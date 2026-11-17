//! Mounted panels: preparing the shell for mount changes, their values
//! and callbacks.
use super::messages::{
    MountApplied, MountCancel, PanelsOp, SetMount, UiCommand, UiReport, UiSnapshot, WindowFacts,
    WindowOp,
};
use super::outbox::MountOp;
use super::*;
use crate::ui::sidebar_tabs::Column;

/// What `apply_mounts` decided for one column of tabs.
struct ColumnPlan {
    /// Its tabs in tab order.
    order: Vec<u64>,
    /// The tab that will be current.
    active: Option<u64>,
    /// The current tab is leaving.
    current_left: bool,
    /// It draws its tab strip.
    tabbed: bool,
}

/// A column's current tab for the host (`Host::current_in`, the start-up
/// rule).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Current {
    /// This tab (`None`: the column has none).
    Tab(Option<u64>),
    /// Start-up has not settled and the column's remembered tab has not
    /// mounted yet: no tab is current.
    Awaits,
}
impl Current {
    pub(crate) fn tab(self) -> Option<u64> {
        match self {
            Current::Tab(tab) => tab,
            Current::Awaits => None,
        }
    }
    pub(crate) fn awaits(self) -> bool {
        self == Current::Awaits
    }
}

/// The wire name of a panel key's slot.
fn slot_of(key: u64) -> &'static str {
    [
        composition::Slot::Bottom,
        composition::Slot::Sidebar,
        composition::Slot::Backdrop,
        composition::Slot::Adjust,
    ]
    .into_iter()
    .find(|&slot| panel_key(panel_plugin(key), slot) == key)
    .map_or("bottom", composition::Slot::name)
}

/// The policy's slot of a column.
pub(crate) fn policy_column(column: Column) -> window_state::Slot {
    match column {
        Column::Sidebar => window_state::Slot::Sidebar,
        Column::Adjust => window_state::Slot::Adjust,
    }
}

/// Initial properties of a panel or settings page are converted when the
/// view is built. A value over the limits is left out now (the property
/// keeps its default) and returned, to be reported (`report_refused`),
/// rather than failing the mount: a mount failing during activation would
/// fail the whole plugin.
pub(crate) fn refuse_initial(properties: &mut BTreeMap<String, Value>) -> Vec<RpcError> {
    let mut refused = Vec::new();
    properties.retain(|name, value| match RpcError::check_property(name, value) {
        Ok(()) => true,
        Err(error) => {
            refused.push(error);
            false
        }
    });
    refused
}

/// The batching window (K6): mounts arriving together (the plugins'
/// activations at start-up) are applied as one change.
const MOUNT_WINDOW: Duration = Duration::from_millis(32);

impl Core {
    /// The hand-over (K6; plan step S2c(3); S3a(3): pushed at the end of the
    /// core's turn, `UiCommand::HandOver`, where the interface took it
    /// before, `Take::Mount`): its oldest accepted mount change, handed to
    /// the interface's empty batching window, numbered. The core records
    /// the change (`Core::handed`, plan step S2d(2)) and keeps the window's
    /// change and when it opened (`Core::window`): it closes the window
    /// itself 32 ms later (S3a(3) review item 6, [`Core::close_window`]).
    /// After a failed batch the window opens due at once, as the interface
    /// took the batch's first change again before. Its `ui.preparing` goes
    /// out here, after the hand-over is queued: a full pipe that stops the
    /// plugin right there cancels the change (`stop_run`), and that cancel
    /// then follows the hand-over in the queue.
    fn hand_over(&mut self) {
        let Some(change) = self.mount_queue.pop_front() else {
            return;
        };
        self.handover_seq += 1;
        let seq = self.handover_seq;
        let mut handed = HandedMount::of(&change);
        handed.seq = seq;
        self.handed = Some(handed);
        let now = Instant::now();
        let since = if std::mem::take(&mut self.retry_due) {
            now.checked_sub(MOUNT_WINDOW).unwrap_or(now)
        } else {
            now
        };
        self.window = Some((change.clone(), since));
        let preparing = unannounced([&change]);
        self.ui_command(UiCommand::HandOver { seq, change });
        if !preparing.is_empty() {
            self.mounts_preparing(preparing);
        }
    }
    /// The batching window closes (S3a(3) review item 6: on the core's
    /// clock at the end of its turn, 32 ms after the hand-over; the
    /// interface closed it and asked, `Take::Close`, before): the window's
    /// change, then every change accepted meanwhile unless after a failed
    /// batch (`mount_singly`), those announced (`ui.preparing`), then only
    /// the ones whose plugin still runs or is held, pushed as one batch.
    /// The batch stays pending in the core (`batch_in_flight`, review B2)
    /// until the interface reports its outcome (`MountApplied` per change,
    /// or `MountRetry`); no change is handed over meanwhile (review B1).
    pub(crate) fn close_window(&mut self) {
        let Some((change, _)) = self.window.take() else {
            return;
        };
        // Its change is applied now, not held, and every cancel's wait ends
        // (the interface's `Batching(None)` before plan step S3a(3)).
        self.handed = None;
        self.cancelled_handovers.clear();
        let mut changes = vec![change];
        if self.mount_singly > 0 {
            self.mount_singly -= 1;
        } else {
            let joining = self.mount_queue.drain(..).collect::<Vec<_>>();
            let preparing = unannounced(&joining);
            if !preparing.is_empty() {
                self.mounts_preparing(preparing);
            }
            changes.extend(joining);
        }
        // A mount of a plugin stopped since is dropped, as before; a tab
        // replayed from a recording belongs to a plugin in standby, which
        // has no process (dropped when it is no longer held either).
        let changes = changes
            .into_iter()
            .filter(|change| {
                let owner = change.reply.owner;
                match &change.contribution {
                    None => true,
                    Some(mount) if mount.recorded => {
                        self.plugins.contains_key(&owner) || self.on_demand.held(owner).is_some()
                    }
                    Some(_) => self.plugins.contains_key(&owner),
                }
            })
            .collect::<Vec<_>>();
        self.batch_in_flight = changes.iter().map(HandedMount::of).collect();
        self.ui_command(UiCommand::Batch(changes));
    }
    /// The interface reported a change of the batch in flight applied
    /// (`MountApplied`, one per change): it is pending no longer.
    pub(crate) fn batch_applied(&mut self, reply: &Reply, key: u64) {
        self.batch_in_flight.retain(|held| {
            !(held.key == key && held.reply.owner == reply.owner && held.reply.id == reply.id)
        });
    }
    /// The interface's batch failed with more than one change
    /// (`UiInput::MountRetry`): they go back to the front of the queue, in
    /// their order, and are tried one at a time, the first at once.
    pub(crate) fn batch_retry(&mut self, changes: Vec<MountChange>) {
        self.batch_in_flight.clear();
        self.mount_singly = changes.len();
        self.retry_due = true;
        self.retry_changes = changes
            .iter()
            .map(|c| (c.reply.owner, c.reply.id, c.key))
            .collect();
        for change in changes.into_iter().rev() {
            self.mount_queue.push_front(change);
        }
    }
    /// The core cancels the change the interface's batching window holds
    /// (plan step S2d(2)): its record and the window go now, and the
    /// interface drops the change when `UiCommand::CancelMount` reaches it.
    /// Plan step S3a(3) (the S3a brief's risk 3): the cancelled change's
    /// number is kept until the interface reports its window empty, so a
    /// report of that change the interface sent before the cancel reached
    /// it (`Batching(Some)`) does not bring the record back
    /// ([`Core::batching`]); no change is handed over meanwhile.
    pub(crate) fn cancel_handed(&mut self, cancel: MountCancel) {
        if let Some(handed) = self.handed.take() {
            self.cancelled_handovers.push(handed.seq);
        }
        self.window = None;
        self.ui_command(UiCommand::CancelMount(cancel));
    }
    /// The core's side of `UiReport::Batching` (plan step S2d(2)): the
    /// change the interface's window holds now, or none. Plan step S3a(3):
    /// the core records the window itself at the hand-over and its close;
    /// a report of the window it holds confirms it, one of a change the
    /// core cancelled or closed since is stale and dropped, and the
    /// interface's report of an empty window (a cancelled change dropped)
    /// ends every cancel's wait.
    pub(crate) fn batching(&mut self, held: Option<HandedMount>) {
        match held {
            Some(held) => {
                let current = self.window.is_some()
                    && self
                        .handed
                        .as_ref()
                        .is_some_and(|handed| handed.seq == held.seq);
                if current && !self.cancelled_handovers.contains(&held.seq) {
                    self.handed = Some(held);
                }
            }
            None => self.cancelled_handovers.clear(),
        }
    }
    /// The changes of the batching window and of the batch in flight, as
    /// the core recorded them (S3a(3) review B2: a batch in flight is
    /// pending until its outcome).
    pub(crate) fn handed_all(&self) -> impl Iterator<Item = &HandedMount> {
        self.handed.iter().chain(self.batch_in_flight.iter())
    }
    /// A change of panel `key` waits: in the core's queue, in the batching
    /// window, or in the batch in flight.
    pub(crate) fn mount_pending(&self, key: u64) -> bool {
        self.mount_queue.iter().any(|c| c.key == key) || self.handed_all().any(|h| h.key == key)
    }
    /// The single-change retry of a failed batch ends once none of the
    /// changes it gave back waits any more (S3a(3) re-review: a change
    /// dropped with its plugin must not leave the retry state to the next
    /// unrelated batch).
    fn retry_ends(&mut self) {
        if self.retry_changes.is_empty() {
            return;
        }
        let waits = |(owner, id, key): &(u64, u64, u64)| {
            let same =
                |reply: &Reply, k: u64| reply.owner == *owner && reply.id == *id && k == *key;
            self.mount_queue.iter().any(|c| same(&c.reply, c.key))
                || self
                    .window
                    .as_ref()
                    .is_some_and(|(c, _)| same(&c.reply, c.key))
                || self.batch_in_flight.iter().any(|h| same(&h.reply, h.key))
        };
        let left = self
            .retry_changes
            .iter()
            .filter(|change| waits(change))
            .copied()
            .collect::<Vec<_>>();
        self.retry_changes = left;
        if self.retry_changes.is_empty() {
            self.mount_singly = 0;
            self.retry_due = false;
        }
    }
    /// A plugin's mount of panel `key` is refused `busy` (S3a(3) re-review
    /// F1): a change of the panel waits in the queue or the window, or one
    /// in the batch in flight other than the host's own removal of it (a
    /// placeholder pruned at the plugin's ready, an id 0 unmount), behind
    /// which the mount queues, as behind that removal applied in place
    /// before the batch was in flight.
    pub(crate) fn mount_blocked(&self, key: u64) -> bool {
        self.mount_queue.iter().any(|c| c.key == key)
            || self.handed.as_ref().is_some_and(|h| h.key == key)
            || self
                .batch_in_flight
                .iter()
                .any(|h| h.key == key && !h.removes(key))
    }
    /// No mount change waits anywhere (the queue, the window, the batch in
    /// flight; start-up settling reads it, `open_files_tick`).
    pub(crate) fn mounts_idle(&self) -> bool {
        self.mount_queue.is_empty()
            && self.window.is_none()
            && self.handed.is_none()
            && self.batch_in_flight.is_empty()
    }
    /// The end of the core's turn (plan step S3a(3)): the batching window
    /// closes when it is due (review item 6); the oldest accepted mount
    /// change is handed to the window when it is empty, no batch is in
    /// flight (review B1) and no cancel waits for the window to report
    /// itself empty (the interface took it at its mount step in the same
    /// tick before, `Take::Mount`; a push keeps that tick); then the table
    /// snapshots and the window settings that changed (the interface pulled
    /// them at its points before, `Take::Tables`, `Take::WindowSettings`).
    pub(crate) fn turn_end(&mut self) {
        self.window_state_out();
        self.retry_ends();
        if !self.headless {
            if self
                .window
                .as_ref()
                .is_some_and(|(_, since)| since.elapsed() >= MOUNT_WINDOW)
            {
                self.close_window();
            }
            if self.window.is_none()
                && self.batch_in_flight.is_empty()
                && self.cancelled_handovers.is_empty()
                && !self.mount_queue.is_empty()
            {
                self.hand_over();
            }
        }
        for snapshot in self.table_snapshots() {
            self.ui_command(UiCommand::Snapshot(snapshot));
        }
        for command in self.window_settings() {
            self.ui_command(command);
        }
    }
    /// The core's own changes of its mount registry, each recorded for the
    /// interface's copy (`MountOp`; plan step S3a(1)).
    pub(crate) fn mount_remove(&mut self, key: u64) -> Option<Mount> {
        let removed = self.mounts.remove(&key);
        if removed.is_some() {
            self.outbox.mount_ops.push(MountOp::Remove(key));
        }
        removed
    }
    /// Panel `from` moves to key `to` of plugin `plugin` (a rekey).
    pub(crate) fn mount_move(&mut self, from: u64, to: u64, plugin: u64) -> bool {
        let Some(mut mount) = self.mounts.remove(&from) else {
            return false;
        };
        mount.spec.owner = to;
        mount.plugin = plugin;
        self.mounts.insert(to, mount);
        self.outbox
            .mount_ops
            .push(MountOp::Move { from, to, plugin });
        true
    }
    /// Panel `key` stays as a placeholder (its plugin parked).
    pub(crate) fn mount_recorded(&mut self, key: u64) {
        if let Some(mount) = self.mounts.get_mut(&key) {
            mount.recorded = true;
            self.outbox.mount_ops.push(MountOp::Recorded(key));
        }
    }
}

impl Host {
    /// The interface's copy of the mount registry follows a change the core
    /// made itself (`MountOp`; plan step S3a(1)); the values its panels
    /// were given since (`properties`, `visible`) stay.
    pub(crate) fn mount_op(&mut self, op: MountOp) {
        match op {
            MountOp::Remove(key) => {
                self.mounts.remove(&key);
            }
            MountOp::Move { from, to, plugin } => {
                if let Some(mut mount) = self.mounts.remove(&from) {
                    mount.spec.owner = to;
                    mount.plugin = plugin;
                    self.mounts.insert(to, mount);
                }
            }
            MountOp::Recorded(key) => {
                if let Some(mount) = self.mounts.get_mut(&key) {
                    mount.recorded = true;
                }
            }
        }
    }
}

/// What the core records of the mount change the interface's batching
/// window holds (plan step S2d(2); `Core::handed`): enough for the core's
/// own checks, which read it instead of the interface's window (a change
/// still waiting there, the slot it claims, the call a recording keeps,
/// the host's own removal of a panel).
#[derive(Clone)]
pub(crate) struct HandedMount {
    pub(crate) reply: Reply,
    /// The panel (`panel_key`).
    pub(crate) key: u64,
    /// A mount's slot; `None`: an unmount.
    pub(crate) slot: Option<composition::Slot>,
    /// A mount's call as a recording keeps it (`Mount::call`); `Null` for
    /// an unmount.
    pub(crate) call: Value,
    /// The hand-over's number (`UiCommand::HandOver`; plan step S3a(3));
    /// 0 for a change not handed over that way.
    pub(crate) seq: u64,
}
impl HandedMount {
    pub(crate) fn of(change: &MountChange) -> Self {
        Self {
            seq: 0,
            reply: change.reply.clone(),
            key: change.key,
            slot: change.contribution.as_ref().map(|m| m.spec.slot),
            call: change
                .contribution
                .as_ref()
                .map_or(Value::Null, |m| m.call.clone()),
        }
    }
    /// The host's own removal of panel `key` (an unmount with id 0), the
    /// change a plugin's own mount of that panel replaces.
    pub(crate) fn removes(&self, key: u64) -> bool {
        self.key == key && self.reply.id == 0 && self.slot.is_none()
    }
    /// The same change (the unit tests).
    #[cfg(test)]
    pub(crate) fn same(&self, other: &Self) -> bool {
        self.reply.owner == other.reply.owner
            && self.reply.id == other.reply.id
            && self.key == other.key
            && self.slot == other.slot
            && self.call == other.call
    }
}

/// The changes `UiReport::MountPreparing` names: those not announced before
/// (a failed batch's changes come back announced).
fn unannounced<'a>(changes: impl IntoIterator<Item = &'a MountChange>) -> Vec<(Reply, u64)> {
    changes
        .into_iter()
        .filter(|change| !change.announced)
        .map(|change| (change.reply.clone(), change.key))
        .collect()
}

impl Host {
    /// The interface's mount batching window and the apply (K6, plan step
    /// S2c(3)). The core hands over its accepted changes; the interface
    /// reports `UiReport::MountApplied` once a batch was applied (R13: the
    /// core applies the state it carries, then answers). Plan step S3a(3):
    /// the core hands the empty window a change at the end of its turn
    /// (`UiCommand::HandOver`, [`Host::hand_over`]) and closes it itself 32
    /// ms later, joining the changes accepted meanwhile (review item 6:
    /// `Core::close_window`, a mount's reply one turn sooner than the
    /// interface's close and ask); the interface applies the batch it
    /// pushes (`UiCommand::Batch`, [`Host::apply_batch`]). A short wait, so
    /// that mounts arriving together (the plugins' activations at start-up)
    /// are applied as one change; nothing waits for plugins that have not
    /// asked yet: each panel compiles on its own when it is first shown
    /// (`ui/panels.rs`). The core writes the `ui.preparing` lines itself
    /// (R6), at the hand-over and at the join.
    pub(crate) fn prepare_mount_tick(&mut self) {
        // The core answers what it held first (plan step S2f(1)): only
        // while it holds something (plan step S3a(2), `TickFacts`).
        if self.tick_facts.held_mounts {
            self.ui_report(UiReport::Input(messages::UiInput::MountTick));
        }
    }
    /// The core's hand-over (plan step S3a(3)): the empty window opens on
    /// the change. The core hands over only to an empty window (S3a(3)
    /// review B1).
    pub(crate) fn hand_over(&mut self, seq: u64, change: MountChange) {
        debug_assert!(self.preparing.is_none(), "the batching window is empty");
        self.handover_seq = seq;
        self.set_preparing(Some((Instant::now(), change)));
    }
    /// The batch the core closed the window on (plan step S3a(3)), joined
    /// and filtered by the core, applied. The window is empty again (the
    /// core knows: it closed it).
    pub(crate) fn apply_batch(&mut self, changes: Vec<MountChange>) {
        let started = Instant::now();
        self.preparing = None;
        if changes.is_empty() {
            return;
        }
        let mut next = self.mounts.clone();
        for change in &changes {
            let owner = change.reply.owner;
            if let Some(mount) = &change.contribution {
                // Mounted after all: an earlier refusal no longer applies
                // (the core's, plan step S3a(1)).
                self.ui_report(UiReport::Input(messages::UiInput::Mounted(owner)));
                next.insert(change.key, mount.clone());
            } else {
                next.remove(&change.key);
            }
        }
        let batch = changes.iter().map(|c| c.key).collect::<Vec<_>>();
        let result = self.apply_mounts(next, &batch);
        self.log_prepared(changes.len(), started, result.is_ok());
        if result.is_err() && changes.len() > 1 {
            // Again one at a time, from the next tick on: the first (at
            // once), then the rest in their order. The queue and the window
            // are the core's (S3a(3) review B1, item 6): every change goes
            // back to its front, announced (no `ui.preparing` again), and
            // the core tries them singly (`Core::batch_retry`).
            let changes = changes
                .into_iter()
                .map(|mut change| {
                    change.announced = true;
                    change
                })
                .collect();
            self.ui_report(UiReport::Input(messages::UiInput::MountRetry(changes)));
            return;
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        // One report per change, in batch order, each right after the
        // change's own interface work (its restart line): the interleaving
        // of the two is today's, and a reply's effects (an activation that
        // completes, R12) come before the next change's restart line, as
        // before. The first carries the R13 handoff.
        let mut first = true;
        for change in changes {
            // A panel kept over its plugin's restart, mounted again (from
            // its compiled file when that did not change) or removed: its
            // restart line goes (`panel_restart.rs`).
            if result.is_ok() && self.restart_waiting(change.key) {
                if let Some(mount) = &change.contribution {
                    diag!(
                        "shell: {} mounted its {} panel again after its restart ({})",
                        mount.manifest,
                        mount.spec.slot.name(),
                        if self.compiled_now.contains(&change.key) {
                            "compiled"
                        } else {
                            "no compile"
                        }
                    );
                }
                self.ui_report(UiReport::Input(messages::UiInput::PanelBack(change.key)));
            }
            let placeholders = change
                .contribution
                .as_ref()
                .filter(|mount| mount.recorded)
                .map(|_| change.key)
                .into_iter()
                .collect();
            let handoff = if std::mem::take(&mut first) {
                self.facts_handoff()
            } else {
                None
            };
            self.ui_report(UiReport::MountApplied(MountApplied {
                batch: vec![(change.reply, change.key)],
                result: result.clone(),
                elapsed_ms: elapsed,
                handoff,
                placeholders,
            }));
        }
    }
    /// The interface's batching window now holds `preparing` (plan step
    /// S2d(2)): the core is told which change it holds, or none
    /// (`UiReport::Batching`), and reads that record (`Core::handed`)
    /// instead of the window.
    pub(crate) fn set_preparing(&mut self, preparing: Option<(Instant, MountChange)>) {
        // Numbered with the hand-over the window took (plan step S3a(3); a
        // failed batch's first change, taken again, keeps the last number).
        let seq = self.handover_seq;
        let held = preparing.as_ref().map(|(_, change)| HandedMount {
            seq,
            ..HandedMount::of(change)
        });
        self.preparing = preparing;
        self.ui_report(UiReport::Batching(held));
    }
    /// The interface's side of `UiCommand::CancelMount` (plan step S2d(2)):
    /// the change the batching window holds is dropped when it is the one
    /// the core cancels.
    pub(crate) fn cancel_mount(&mut self, cancel: MountCancel) {
        let dropped = self
            .preparing
            .as_ref()
            .is_some_and(|(_, change)| match cancel {
                MountCancel::Removal { key } => {
                    change.key == key && change.reply.id == 0 && change.contribution.is_none()
                }
                MountCancel::Owner(owner) => change.reply.owner == owner,
            });
        if dropped {
            self.set_preparing(None);
        }
    }
    /// The R13 handoff of a mount batch and of `layout.set`: the window
    /// facts the interface last reported, with the tabs as the apply left it
    /// (`window_facts_report`). The facts themselves are measured at the
    /// next `window_tick` as before (its `window.state` goes out then);
    /// `None` without a shell.
    pub(crate) fn facts_handoff(&mut self) -> Option<Box<WindowFacts>> {
        let shell = self.shell.as_ref().map(ComponentHandle::clone_strong)?;
        let facts = self.window_facts.clone();
        Some(Box::new(self.window_facts_report(Some(&shell), facts)))
    }
}
impl Core {
    /// The core's side of `UiReport::MountPreparing` (R6): the changes left
    /// the batching queue, so their plugins hear `ui.preparing`
    /// (`phase: preparing`), in order.
    pub(crate) fn mounts_preparing(&mut self, changes: Vec<(Reply, u64)>) {
        for (reply, key) in changes {
            self.send(
                reply.owner,
                json!({"event":"ui.preparing","data":{"phase":"preparing","slot":slot_of(key)}}),
            );
        }
    }
    /// The core's side of `UiReport::MountApplied`, in R13's order: (1) the
    /// state the report carries (the handoff's facts and tab order; the
    /// mount record was made the registry, `Core::mounts`, where the
    /// interface applied it, `apply_mounts`), (2) per change its
    /// `ui.preparing` (`ready` or `failed`; none for a placeholder) and its
    /// reply (a change of the host's own, id 0: a failure is announced as
    /// `host.error`), whose activation completes after it (R12, `respond`),
    /// (3) `window.state` if the facts changed, once for the turn's reports
    /// (`Core::window_state_out`).
    pub(crate) fn mounts_applied(&mut self, applied: MountApplied) {
        let MountApplied {
            batch,
            result,
            elapsed_ms,
            handoff,
            placeholders,
        } = applied;
        if let Some(facts) = handoff {
            self.apply_facts(*facts);
        }
        for (reply, key) in batch {
            // Its outcome: no longer pending (S3a(3) review B2).
            self.batch_applied(&reply, key);
            let owner = reply.owner;
            if !placeholders.contains(&key) {
                self.send(owner,json!({"event":"ui.preparing","data":{"phase":if result.is_ok(){"ready"}else{"failed"},"elapsedMs":elapsed_ms,"slot":slot_of(key)}}));
            }
            if reply.id == 0 {
                if let Err(error) = &result {
                    self.broadcast(
                        "host.error",
                        json!({"code":"ui-owner-cleanup","owner":owner,"message":error.message}),
                    );
                }
            } else {
                self.respond(reply, result.clone());
                // Its plugin stopped while the batch was in flight (S3a(3)
                // re-review F2): its panel gets the end state the stop gave
                // it when the panel was in place before.
                if result.is_ok() && !self.plugins.contains_key(&owner) {
                    self.stopped_owners_mount(owner, key);
                }
            }
        }
    }
    /// A mount of `owner`'s panel `key` applied after `owner` stopped (it
    /// was in the batch in flight when `stop_run` ran, so the stop did not
    /// see it; the interface's record put it into `Core::mounts` since):
    /// what `stop_run` does with a panel in place, for this one. Held in
    /// standby: a placeholder tab (recorded). Otherwise it goes: the record,
    /// the interface's panel (`PanelsOp::Gone`), and an id 0 unmount at the
    /// front of the queue, so its slot is free again.
    pub(crate) fn stopped_owners_mount(&mut self, owner: u64, key: u64) {
        let Some(mount) = self.mounts.get(&key) else {
            return;
        };
        if mount.plugin != owner || mount.recorded {
            return;
        }
        // Held, a tabbed panel stays as a placeholder; a bottom or backdrop
        // panel goes, as the park removes it (S3a(3) re-review S1).
        let slot = mount.spec.slot;
        if self.on_demand.held(owner).is_some() && slot.tabbed() {
            self.mount_recorded(key);
            return;
        }
        self.mount_remove(key);
        self.ui_command(UiCommand::Panels(PanelsOp::Gone(vec![(key, slot)])));
        self.mount_queue.push_front(MountChange {
            reply: Reply { owner, id: 0 },
            key,
            contribution: None,
            announced: false,
        });
    }
}
impl Host {
    /// A command of the core reaches the interface (rule R3), on this
    /// thread until plan step S3a. Plan step S2c(3):
    /// `ui.setMount`, answered from the interface's `UiReport::Answered`
    /// (a mount change is taken by the batching window instead,
    /// `Core::mount_command`); S2c(4): `settings.*`
    /// (`Host::settings_command`, ui/settings_window.rs); S2c(5): the
    /// layout, the palette's lead, the file dialogs, the system
    /// confirmation and the sheet (`Host::layout_command`,
    /// `lead_command`, `dialog_command`, `confirm_command`,
    /// `review_command`); S2c(6): a view's row-patch records dropped
    /// (`ResetWrites`, `ui::ui_image::Images::reset_writes`); S2c(7): the
    /// last `source.state`, the info panels, the palette, the window
    /// settings in effect (`LastSource`, `InfoPanels`, `Theme`,
    /// `CursorAutoHide`, `WindowOp::OnTop`); S2d(1): the view requests
    /// (`View`, `Host::view_command`), the video area and
    /// picture-in-picture (`WindowOp::VideoArea`, `WindowOp::Pip`;
    /// `video_area_command`, `pip_command`). Plan step S2f(3): the core puts
    /// it into its outbox (`Core::ui_command`); the interface carries it
    /// out here where the driver drains it (`Host::drain_outbox`: after the
    /// event, report or step that issued it), followed by its facts.
    pub(crate) fn apply_ui_command(&mut self, command: UiCommand) {
        match command {
            UiCommand::Snapshot(UiSnapshot::LastSource(source)) => self.source = source,
            UiCommand::Snapshot(UiSnapshot::InfoPanels(panels)) => {
                self.info_panels = panels;
                self.show_info_panels();
            }
            UiCommand::Snapshot(UiSnapshot::Theme(theme)) => {
                self.shown_theme = theme;
                self.theme_all();
            }
            UiCommand::Snapshot(UiSnapshot::CursorAutoHide(playing)) => {
                self.cursor_playing = playing;
            }
            UiCommand::Window(WindowOp::OnTop(on)) => self.on_top = on,
            UiCommand::ResetWrites { owner, view } => self.images.reset_writes(owner, &view),
            UiCommand::SetMount(op) => {
                let SetMount {
                    reply,
                    key,
                    params,
                    prepared,
                } = op;
                let result = crate::capabilities::ui_mount::write_mount(
                    self, &reply, key, &params, prepared,
                );
                self.ui_report(UiReport::Answered {
                    reply,
                    result,
                    handoff: None,
                    confirm_failed: false,
                });
            }
            UiCommand::Settings(op) => self.settings_command(op),
            UiCommand::Layout(op) => self.layout_command(op),
            UiCommand::Lead(op) => self.lead_command(op),
            UiCommand::Dialog(request) => self.dialog_command(request),
            UiCommand::Confirm(request) => self.confirm_command(request),
            UiCommand::Review(request) => self.review_command(request),
            UiCommand::View(op) => self.view_command(op),
            UiCommand::Window(WindowOp::VideoArea {
                reply,
                width,
                height,
            }) => self.video_area_command(reply, width, height),
            UiCommand::Window(WindowOp::Pip { reply, on }) => self.pip_command(reply, on),
            // Plan step S2d(2): the core cancels the change the batching
            // window holds; start-up settled.
            UiCommand::CancelMount(cancel) => self.cancel_mount(cancel),
            // Plan step S2d(3): a stopped run's interface part, panels that
            // leave or move, a stopped plugin's waiting sheet.
            UiCommand::RevokeOwner { owner, park: _ } => self.revoke_owner(owner),
            UiCommand::Panels(op) => self.panels_command(op),
            UiCommand::DropConfirm { owner } => self.drop_confirm_of(owner),
            // Plan step S2d(4): the OSD, what `source.state` shows, the
            // window's one-way operations, the palette, a panel's menu.
            UiCommand::Osd(op) => self.osd_command(op),
            UiCommand::Snapshot(UiSnapshot::Source(shown)) => self.show_source(shown),
            UiCommand::Window(
                op @ (WindowOp::Fullscreen(_) | WindowOp::Raise | WindowOp::Quit),
            ) => self.window_op(op),
            UiCommand::OpenPalette => self.open_palette(),
            UiCommand::PanelMenu(menu) => self.panel_menu(menu),
            UiCommand::Snapshot(UiSnapshot::StartupSettled(settled)) => {
                self.startup_panels = settled;
            }
            // Plan step S2f(1) (design §5.3, group B): interface work the
            // core called in place before.
            UiCommand::RefreshMenu => {
                if self.menu_open {
                    let result = self.refresh_menu();
                    self.report_menu(result);
                }
            }
            UiCommand::ShowWaiting => self.show_waiting_confirmation(),
            UiCommand::Activity => self.policy.activity(self.now_ms()),
            UiCommand::Window(WindowOp::ApplyOnTop) => self.apply_on_top(),
            // Plan step S2f(3): the dialogs placed by the interface, and the
            // sheet's offer written again.
            UiCommand::Ask(ask) => self.ask_command(ask),
            UiCommand::OpenDialog { reply, purpose } => self.open_dialog_command(reply, purpose),
            UiCommand::ReviewOffer => self.review_offer_changed(),
            // Plan step S3a(3): the pushed tables and the batching window's
            // answers.
            UiCommand::Snapshot(snapshot @ (UiSnapshot::Registry(_) | UiSnapshot::Bindings(_))) => {
                self.apply_table(snapshot)
            }
            UiCommand::HandOver { seq, change } => self.hand_over(seq, change),
            UiCommand::Batch(changes) => self.apply_batch(changes),
            _ => unreachable!("not sent: `Mount` (S3a(3): `HandOver`), `RekeyOwner`"),
        }
    }
    fn log_prepared(&self, changes: usize, started: Instant, ok: bool) {
        if self.frame_log.is_some() {
            diag!(
                "vestra shell: {} change(s), {} panel(s), {} compiled, prepared in {:.1} ms ({})",
                changes,
                self.mounts.len(),
                self.panels.definitions(),
                started.elapsed().as_secs_f64() * 1000.,
                if ok { "ready" } else { "failed" }
            );
        }
    }
    /// Make `next` the mounted panels (sidebar-tabs.md); `batch`: the panels
    /// whose mount or unmount this applies. The shell is never compiled
    /// again: each panel is compiled on its own, when it is first shown
    /// (`ui/panels.rs`). A panel of the batch that shows at once (the
    /// bottom bar, the backdrop, the tab that will be current in a column
    /// that is wanted shown) is compiled here, before anything changes, so
    /// one that does not compile fails its mount as before; a tab mounted
    /// in the background compiles nothing (but an adjust tab joining the
    /// front tabs of the column wanted shown: right after, `preload_front`),
    /// and one that turns out not to compile when it is first shown is
    /// reported to its plugin then (`show_panel`). The policy, the panel
    /// keys and the remembered state follow.
    pub(crate) fn apply_mounts(
        &mut self,
        next: BTreeMap<u64, Mount>,
        batch: &[u64],
    ) -> Result<Value, RpcError> {
        let started = Instant::now();
        // A new mount gets a new chance to compile.
        for owner in batch {
            self.tabs_failed.remove(owner);
        }
        self.tabs_failed.retain(|owner| next.contains_key(owner));
        // Gone, or moved to another slot (no tab left behind).
        let leaving = self
            .mounts
            .iter()
            .filter(|(owner, mount)| {
                next.get(owner)
                    .is_none_or(|m| m.spec.slot != mount.spec.slot)
            })
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>();
        // The tab that will be current in each column, so the first frame
        // shows it: the policy's after the leaving tabs went (the next one
        // takes over a removed current tab). A placeholder that would take
        // over that way is passed over for a running plugin's tab, so that
        // removing a tab never wakes a neighbour unasked.
        let mut planned = self.policy.clone();
        for owner in &leaving {
            planned.remove_owner(*owner);
        }
        let recorded = |owner: u64| next.get(&owner).is_some_and(|m| m.recorded);
        let plans = Column::ALL.map(|column| {
            let order = self.tab_order(column, &next);
            let remembered = self.column_memory(column).active;
            let current_left = self
                .policy
                .active(policy_column(column))
                .is_some_and(|owner| leaving.contains(&owner));
            let active = sidebar_tabs::next_active(
                &order,
                planned.active(policy_column(column)),
                current_left,
                recorded,
            )
            .or_else(|| {
                order
                    .iter()
                    .copied()
                    .find(|o| next.get(o).map(|m| m.manifest.as_str()) == remembered.as_deref())
            })
            .or_else(|| order.first().copied());
            let tabbed = if column.always_tabbed() {
                !order.is_empty()
            } else {
                order.len() > 1
            };
            ColumnPlan {
                order,
                active,
                current_left,
                tabbed,
            }
        });
        let titles = plans
            .iter()
            .map(|plan| {
                ui_values::from_json(&json!(
                    plan.order
                        .iter()
                        .filter_map(|owner| next.get(owner).map(|m| m.title.clone()))
                        .collect::<Vec<_>>()
                ))
                .map_err(|error| RpcError::new("ui-prepare", error.to_string()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let tabbed = [plans[0].tabbed, plans[1].tabbed];
        // The batch's mounts (a placeholder replayed from a recording has
        // no process and draws nothing).
        let mounted = batch
            .iter()
            .filter_map(|key| next.get(key))
            .filter(|mount| !mount.recorded)
            .cloned()
            .collect::<Vec<_>>();
        // The ones that show at once compile first: a failure leaves
        // everything as it was.
        self.compiled_now.clear();
        self.panels.clear_pending();
        for mount in &mounted {
            let shows = match Column::of(mount.spec.slot) {
                None => true,
                // The tab current for the host once `next` is mounted
                // (`current_in`: none while the column waits for its
                // remembered tab).
                Some(column) => {
                    self.column_wanted(column)
                        && self
                            .current_in(column, plans[column.index()].active, &next)
                            .tab()
                            == Some(mount.spec.owner)
                }
            };
            if !shows {
                continue;
            }
            let compiled = self.panels.precompile(&mount.spec);
            if !matches!(compiled, Ok(None)) {
                self.log_compile(mount, compiled.as_ref().ok().copied().flatten());
            }
            match compiled {
                Ok(Some(_)) => {
                    self.compiled_now.insert(mount.spec.owner);
                }
                Ok(None) => {}
                Err(error) => return Err(RpcError::new("ui-prepare", error.to_string())),
            }
        }
        // From here on nothing fails. Each mount of the batch is a new
        // panel with the values it was mounted with (the panel it replaces,
        // a placeholder's kept one or one kept over its plugin's restart,
        // goes with its instances); panels no longer mounted go.
        let mut failures = Vec::new();
        for mount in &mounted {
            let panel = self.panels.insert(mount.spec.clone());
            for (name, value) in self.mount_values(mount) {
                if let Err(error) = panel.write(&name, value) {
                    failures.push((mount.spec.owner, format!("{name}: {error:#}")));
                }
            }
            if let Some(slot) = policy_slot(mount.spec.slot)
                && self.mounts.contains_key(&mount.spec.owner)
                && let Err(error) =
                    self.policy
                        .set_locks(slot, mount.spec.owner, window_state::Locks::default())
            {
                diag!("mount policy reset: {error}");
            }
        }
        self.report_value_failures(failures);
        self.panels.clear_pending();
        self.panels.retain(|key| next.contains_key(&key));
        for old in &leaving {
            self.policy.remove_owner(*old);
            if !next.contains_key(old)
                && let Some(mount) = self.mounts.get(old)
            {
                self.images
                    .release_view(mount.plugin, mount_view(mount.spec.slot));
            }
        }
        let memory = Column::ALL.map(|column| self.column_memory(column));
        let attach = next
            .values()
            .filter(|m| !m.spec.slot.tabbed())
            .chain(
                plans
                    .iter()
                    .flat_map(|plan| plan.order.iter().filter_map(|owner| next.get(owner))),
            )
            .filter(|m| !self.mounts.contains_key(&m.spec.owner) || leaving.contains(&m.spec.owner))
            .map(|m| (m.spec.owner, m.spec.slot, m.visible, m.manifest.clone()))
            .collect::<Vec<_>>();
        for (id, mount_slot, mount_visible, manifest) in attach {
            let Some(policy) = policy_slot(mount_slot) else {
                continue;
            };
            let column = Column::of(mount_slot);
            let (visible, detached) = match column {
                Some(column) => {
                    let memory = &memory[column.index()];
                    (memory.visible, memory.detached)
                }
                None => (mount_visible, false),
            };
            if let Err(error) = self.policy.attach(policy, id, visible, detached) {
                diag!("mount policy attach: {error}");
            }
            // The tab that was current at the last run is current again.
            if let Some(column) = column
                && memory[column.index()].active.as_deref() == Some(&manifest)
            {
                let _ = self.policy.set_active(id);
            }
        }
        for (column, plan) in Column::ALL.into_iter().zip(&plans) {
            // The tab chosen above, when the current one left.
            if plan.current_left
                && let Some(active) = plan.active
            {
                let _ = self.policy.set_active(active);
            }
            self.policy.order_column(policy_column(column), &plan.order);
        }
        // The tabs as the batch left them reach the core (rule R9).
        self.report_tabs();
        self.policy.activity(self.now_ms());
        {
            let mut input = self.input.borrow_mut();
            input.bottom = next
                .values()
                .find(|m| m.spec.slot == composition::Slot::Bottom)
                .map(|m| m.plugin);
        }
        // The mount record of the R13 handoff, given to the core's registry
        // here, in place, before any reply (`mounts_applied`): what follows
        // (the panel keys, the panels shown) reads the record it makes. Plan
        // step S3a(1): the interface's copy is the record; the core's
        // registry takes it without the values the panels were given, which
        // only the interface reads (`UiInput::Mounts`).
        let record = next
            .iter()
            .map(|(key, mount)| (*key, mount.record()))
            .collect();
        self.mounts = next;
        self.ui_report(UiReport::Input(messages::UiInput::Mounts(record)));
        {
            // The panel keys' plugins: their commands take the panel's keys
            // (none while a column waits for its remembered tab).
            let sidebar = self.current_tab(Column::Sidebar).map(panel_plugin);
            let adjust = self.current_tab(Column::Adjust).map(panel_plugin);
            let mut input = self.input.borrow_mut();
            input.sidebar = sidebar;
            input.adjust = adjust;
        }
        self.ui_report(UiReport::Input(messages::UiInput::PanelPlugins));
        for (state, plan) in self.columns.iter_mut().zip(&plans) {
            state.tabs = plan.order.clone();
            state.tabbed = plan.tabbed;
        }
        for column in Column::ALL {
            self.remember_column(column, false);
        }
        // The strips (a tab came or went) and the panels, in the same
        // frame: each column's current tab, the bar and the backdrop.
        for mount in self.mounts.values() {
            if let Some(column) = Column::of(mount.spec.slot)
                && let Some(panel) = self.panels.panel(mount.spec.owner)
            {
                panel.set_tabbed(tabbed[column.index()]);
            }
        }
        if let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) {
            let error = |error: anyhow::Error| RpcError::new("ui-prepare", error.to_string());
            for ((column, titles), plan) in Column::ALL.into_iter().zip(&titles).zip(&plans) {
                let p = column.prefix();
                shell
                    .set_property(&format!("{p}-titles"), titles.clone())
                    .map_err(|e| error(e.into()))?;
                if let Some(index) = plan
                    .active
                    .and_then(|a| plan.order.iter().position(|&o| o == a))
                {
                    shell
                        .set_property(&format!("{p}-active"), SValue::Number(index as f64))
                        .map_err(|e| error(e.into()))?;
                }
            }
            shell
                .set_property("sidebar-tabbed", tabbed[0].into())
                .map_err(|e| error(e.into()))?;
        }
        self.sync_panels();
        Ok(
            json!({"status":"ready","shellGeneration":self.shell_generation,"prepareMs":started.elapsed().as_secs_f64()*1000.}),
        )
    }
    /// Whether a column is wanted shown (also while it is hidden for the
    /// moment: collapsed in fullscreen, detached): its current tab's panel
    /// is compiled and instantiated only then. Before its first panel is
    /// attached: as remembered from the last run.
    pub(crate) fn column_wanted(&self, column: Column) -> bool {
        self.policy.column_state(policy_column(column)).map_or_else(
            || self.column_memory(column).visible,
            |(preferred, _)| preferred,
        )
    }
    /// The start-up rule for a column's current tab, its one source of
    /// truth: while start-up has not settled (`startup_panels`), a column
    /// whose remembered current tab (the last run's) has not mounted yet
    /// has no current tab (`Current::Awaits`), instead of the tab that is
    /// current meanwhile (plugins start one after another, and a
    /// placeholder replayed from a recording mounts at once: a flash of
    /// another tab, a compile for nothing, a wake for nothing). Otherwise
    /// it is `active`, the policy's (once settled, whatever it is: the
    /// remembered plugin may be gone). Of the column with the mounts
    /// `mounts`: `next` while a mount change is planned, else the mounted
    /// ones. Every consumer asks this, through `current` or `current_tab`:
    /// what the column shows and compiles (`apply_mounts`, `sync_panels`),
    /// the floating sidebar, the panel keys, the tab's wake and its
    /// kept-running work (`sync_column`, `tab_demand`) and
    /// `Busy::ShownTab`. The tab strip marks the policy's current tab all
    /// the same (the user's choice, kept).
    fn current_in(
        &self,
        column: Column,
        active: Option<u64>,
        mounts: &BTreeMap<u64, Mount>,
    ) -> Current {
        let awaits = crate::ui::sidebar_tabs::awaits_remembered(
            self.startup_panels,
            self.column_memory(column).active.as_deref(),
            active
                .and_then(|active| mounts.get(&active))
                .map(|mount| mount.manifest.as_str()),
            mounts
                .values()
                .filter(|mount| mount.spec.slot == column.slot())
                .map(|mount| mount.manifest.as_str()),
        );
        if awaits {
            Current::Awaits
        } else {
            Current::Tab(active)
        }
    }
    /// A column's current tab for the host now (`current_in` over the
    /// policy's current tab and the mounted panels).
    pub(crate) fn current(&self, column: Column) -> Current {
        self.current_in(
            column,
            self.policy.active(policy_column(column)),
            &self.mounts,
        )
    }
    /// The tab `current` names: none while the column waits for its
    /// remembered tab (neither a placeholder, which would be woken, nor a
    /// running plugin's, which would be kept running).
    pub(crate) fn current_tab(&self, column: Column) -> Option<u64> {
        self.current(column).tab()
    }
    /// One line per compile of a panel's file (tests/startup_compiles.rs
    /// counts them): a compile blocks the UI thread. `took`: `None` when it
    /// failed.
    fn log_compile(&self, mount: &Mount, took: Option<f64>) {
        diag!(
            "panel: compile [{}:{}]{} at +{} ms ({})",
            mount.manifest,
            mount.spec.slot.name(),
            took.map_or_else(String::new, |ms| format!(" in {ms:.1} ms")),
            self.now_ms(),
            if took.is_some() { "ready" } else { "failed" }
        );
    }
    /// Show panel `key` (`None`, a placeholder or one that does not
    /// compile: nothing) in the container of `slot` of `window`. A panel
    /// first shown compiles now; one that does not compile is left out
    /// until its plugin mounts it again and reported to its plugin
    /// (`ui.error`), so it never fails another plugin's panel. Whether the
    /// container's content changed.
    pub(crate) fn show_panel(
        &mut self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
        slot: composition::Slot,
        key: Option<u64>,
    ) -> bool {
        use crate::ui::panels::Shown;
        let key = key.filter(|key| self.panels.contains(*key) && !self.tabs_failed.contains(key));
        let events = self.mount_events.clone();
        match self.panels.show(window, generation, slot, key, &events) {
            Ok(Shown::Same) => false,
            Ok(Shown::Changed { compiled_ms }) => {
                if let (Some(took), Some(mount)) =
                    (compiled_ms, key.and_then(|key| self.mounts.get(&key)))
                {
                    self.log_compile(mount, Some(took));
                }
                true
            }
            Err(error) => {
                let Some(key) = key else {
                    diag!("panel container {}: {error:#}", slot.name());
                    return false;
                };
                if let Some(mount) = self.mounts.get(&key) {
                    self.log_compile(mount, None);
                }
                diag!("panel {key}: it does not compile: {error:#}");
                self.tabs_failed.insert(key);
                let refusal = RpcError::new("ui-prepare", format!("ui-prepare: {error:#}"));
                self.report_ui_error(panel_plugin(key), &refusal);
                matches!(
                    self.panels.show(window, generation, slot, None, &events),
                    Ok(Shown::Changed { .. })
                )
            }
        }
    }
    /// Compile the front tabs of `column` (`sidebar_tabs::front`) that are
    /// not loaded, without showing them; logged as any compile. One that
    /// does not compile is left out and reported to its plugin, as at its
    /// showing (`show_panel`). A placeholder tab (no panel in this run) is
    /// skipped. Whether anything was compiled.
    fn preload_front(&mut self, column: Column, active: Option<u64>) -> bool {
        let front = crate::ui::sidebar_tabs::front(&self.columns[column.index()].tabs, active);
        let mut loaded = false;
        for key in front {
            if self.tabs_failed.contains(&key) || !self.panels.contains(key) {
                continue;
            }
            match self.panels.load(key) {
                Ok(None) => {}
                Ok(Some(took)) => {
                    loaded = true;
                    if let (Some(took), Some(mount)) = (took, self.mounts.get(&key)) {
                        self.log_compile(mount, Some(took));
                    }
                }
                Err(error) => {
                    if let Some(mount) = self.mounts.get(&key) {
                        self.log_compile(mount, None);
                    }
                    diag!("panel {key}: it does not compile: {error:#}");
                    self.tabs_failed.insert(key);
                    let refusal = RpcError::new("ui-prepare", format!("ui-prepare: {error:#}"));
                    self.report_ui_error(panel_plugin(key), &refusal);
                }
            }
        }
        loaded
    }
    /// Every slot's container shows its panel (each tick, and at once after
    /// a mount change): the bottom bar, the backdrop, and each column's
    /// current tab while the column is wanted shown, in the shell and (the
    /// sidebar) the floating window. Only what changed touches Slint; a
    /// changed container gets its instance before the next frame, and
    /// before the host reads geometry.
    pub(crate) fn sync_panels(&mut self) {
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        use composition::Slot;
        let started = Instant::now();
        let generation = self.shell_generation;
        // The palette a new instance gets.
        self.panels.set_theme(self.theme());
        let floating = self
            .floating
            .as_ref()
            .map(|f| (f.instance.clone_strong(), f.generation));
        match &floating {
            Some((_, floating)) => self.panels.prune(&[generation, *floating]),
            None => self.panels.prune(&[generation]),
        }
        let mut changed = false;
        for slot in [Slot::Bottom, Slot::Backdrop] {
            let key = self
                .mounts
                .values()
                .find(|m| m.spec.slot == slot)
                .map(|m| m.spec.owner);
            changed |= self.show_panel(&shell, generation, slot, key);
        }
        let mounted = SValue::Bool(self.panels.shown(generation, Slot::Bottom).is_some());
        if shell.get_property("bottom-mounted").ok().as_ref() != Some(&mounted) {
            let _ = shell.set_property("bottom-mounted", mounted);
            changed = true;
        }
        if changed {
            view::settle_layout(shell.window());
        }
        self.mirror_bar(&shell);
        for column in Column::ALL {
            let slot = column.slot();
            let active = self.policy.active(policy_column(column));
            let current = self.current(column);
            let wanted = self.column_wanted(column) && !current.awaits();
            // Not wanted shown (or waiting for its remembered tab): what it
            // showed stays (nothing compiles), unless that is no longer its
            // current tab.
            let key = active.filter(|&a| wanted || self.panels.shown(generation, slot) == Some(a));
            let shown = self.show_panel(&shell, generation, slot, key);
            changed |= shown;
            // The adjust column wanted shown has all its front tabs compiled
            // (user ruling 2026-11-11): its first opening compiles them at
            // once, and a tab mounted into the front while it is open
            // compiles at that mount, so a switch never compiles.
            let loaded = column == Column::Adjust && wanted && self.preload_front(column, active);
            if shown || loaded {
                self.panels.evict(slot);
            }
            // The floating sidebar shows its current tab whatever it showed
            // (none while the column waits for its remembered tab).
            if column == Column::Sidebar
                && let Some((window, floating)) = &floating
                && self.show_panel(window, *floating, slot, current.tab())
            {
                view::settle_layout(window.window());
                self.panels.evict(slot);
            }
        }
        if changed {
            view::settle_layout(shell.window());
            // `VESTRA_FRAME_LOG`: what a container change cost on the UI
            // thread (a compile, an instance, or the kept one embedded again).
            if self.frame_log.is_some() {
                diag!(
                    "vestra panels: containers changed in {:.1} ms",
                    started.elapsed().as_secs_f64() * 1000.
                );
            }
        }
        let refused = self.panels.take_refused();
        if !refused.is_empty() {
            self.report_value_failures(refused);
        }
        self.sync_restart_lines();
    }
    /// The bottom bar's reported width and `placed` to the shell's strip
    /// (`panels::mirror_bar`): a change shows there one tick later at most.
    fn mirror_bar(&self, shell: &slint_interpreter::ComponentInstance) {
        let bar = self.panels.bar_instance(self.shell_generation);
        crate::ui::panels::mirror_bar(shell, bar.as_ref().map(|(_, bar)| bar));
    }
    /// Values a new instance could not take (a property the panel does not
    /// declare, a value of the wrong type): logged and reported to their
    /// plugin (`ui.error`, its `onError`), as `mount_values` does; the panel
    /// is drawn with its defaults for them.
    pub(crate) fn report_value_failures(&mut self, failures: Vec<(u64, String)>) {
        for (owner, message) in failures {
            diag!("mount property {owner}/{message}");
            self.report_ui_error(panel_plugin(owner), &RpcError::new("ui", message));
        }
    }
}
impl Core {
    /// Initial properties `refuse_initial` left out: logged and reported
    /// to the plugin (`ui.error`, its `onError`).
    pub(crate) fn report_refused(&mut self, owner: u64, refused: Vec<RpcError>) {
        for error in refused {
            diag!("initial property {owner}: {}", error.message);
            self.send(owner, ui_error(&error));
        }
    }
}
impl Host {
    /// A mount's properties converted for a new instance. They were checked
    /// when written, so a value that no longer converts is an image file
    /// removed since (or a document over the text total): it is left out (the
    /// property keeps its default), logged and reported to the plugin
    /// (`ui.error`, its `onError`).
    pub(crate) fn mount_values(&mut self, mount: &Mount) -> Vec<(String, SValue)> {
        let owner = mount.plugin;
        let view = mount_view(mount.spec.slot);
        let mut values = Vec::new();
        for (name, value) in &mount.properties {
            match self.prop_value(owner, view, name, value) {
                Ok(value) => values.push((name.clone(), value)),
                Err(error) => {
                    diag!("mount property {owner}/{name}: {}", error.message);
                    self.report_ui_error(owner, &error);
                }
            }
        }
        values
    }
    pub(crate) fn mount_callbacks(&mut self) {
        for event in self.mount_input.try_iter().take(64).collect::<Vec<_>>() {
            // A callback names the key its panel was mounted under: one
            // whose plugin woke or restarted under another owner since is
            // no longer mounted under it, and is dropped.
            if !self.window_generation_live(event.shell_generation)
                || !self.mounts.contains_key(&event.owner)
            {
                continue;
            }
            // Its process is gone: dropped, never held for the next run
            // (a stale click). The panel takes no pointer meanwhile.
            if self.restart_waiting(event.owner) {
                diag!(
                    "mount callback {} of a panel waiting for its plugin's restart: dropped",
                    event.callback
                );
                continue;
            }
            // To the panel's plugin: `view` stays `"$mount"` for SDKs before
            // 1.6.15 (one panel each); `slot` says which panel.
            let (owner, slot) = (panel_plugin(event.owner), slot_of(event.owner));
            // The main window's panel, or the floating sidebar's.
            self.acted_in(owner, event.shell_generation);
            // The core writes the plugin's line (plan step S2f(1)).
            let line = match event.args {
                Ok(args) => messages::PluginLine::UiCallback {
                    owner,
                    data: json!({"view":"$mount","slot":slot,"callback":event.callback,"args":args}),
                },
                Err(error) => messages::PluginLine::UiError {
                    owner,
                    data: json!({"code":"ui","slot":slot,"message":format!("callback {}: {error}",event.callback)}),
                },
            };
            self.report_line(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SDK 1.6.15: a plugin mounts one panel per slot. Each panel has a key
    /// of its own (the shell, the policy and the compile sets know it by
    /// that); the plugin and the slot come back from it, and keys of
    /// different plugins or slots never meet.
    #[test]
    fn each_panel_of_a_plugin_has_its_own_key() {
        use composition::Slot;
        let slots = [Slot::Bottom, Slot::Sidebar, Slot::Backdrop, Slot::Adjust];
        let mut seen = std::collections::BTreeSet::new();
        for owner in [1, 2, 7, 1000] {
            for slot in slots {
                let key = panel_key(owner, slot);
                assert!(seen.insert(key), "{owner} {slot:?}");
                assert_eq!(panel_plugin(key), owner);
                assert_eq!(slot_of(key), slot.name());
            }
        }
        // Images and row patches per panel: the bottom bar keeps `$mount`.
        assert_eq!(mount_view(Slot::Bottom), "$mount");
        let views = slots.map(mount_view);
        assert_eq!(
            views
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            4
        );
    }
}

/// Plan step S2d(2): the core's record of the change the batching window
/// holds.
#[cfg(test)]
mod handed_tests {
    use super::*;

    fn held(id: u64, slot: Option<composition::Slot>) -> HandedMount {
        HandedMount {
            reply: Reply { owner: 7, id },
            key: 28,
            slot,
            call: Value::Null,
            seq: 0,
        }
    }

    /// Only the host's own unmount (id 0) of that panel gives way to the
    /// plugin's mount; a plugin's unmount or any mount does not.
    #[test]
    fn only_the_hosts_own_unmount_of_the_panel_is_a_removal() {
        assert!(held(0, None).removes(28));
        assert!(!held(0, None).removes(29));
        assert!(!held(5, None).removes(28));
        assert!(!held(0, Some(composition::Slot::Sidebar)).removes(28));
    }

    #[test]
    fn records_are_the_same_only_when_every_field_is() {
        let base = held(3, Some(composition::Slot::Sidebar));
        assert!(base.same(&base.clone()));
        assert!(!base.same(&held(4, Some(composition::Slot::Sidebar))));
        assert!(!base.same(&held(3, None)));
        let mut called = base.clone();
        called.call = json!({"slot":"sidebar"});
        assert!(!base.same(&called));
    }
}
