//! Mounted panels: preparing the shell for mount changes, their values
//! and callbacks.
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

impl Host {
    pub(crate) fn prepare_mount_tick(&mut self) {
        self.release_held_mounts();
        if self.preparing.is_none() {
            if let Some(change) = self.mount_queue.pop_front() {
                if !change.announced {
                    self.send(
                        change.reply.owner,
                        json!({"event":"ui.preparing","data":{"phase":"preparing","slot":slot_of(change.key)}}),
                    );
                }
                self.preparing = Some((Instant::now(), change));
            }
            return;
        }
        // A short wait, so that mounts arriving together (the plugins'
        // activations at start-up) are applied as one change. Nothing waits
        // for plugins that have not asked yet: the shell never compiles
        // again, and each panel compiles on its own when it is first shown
        // (`ui/panels.rs`), so a late mount costs only its own panel.
        let waited = self.preparing.as_ref().unwrap().0.elapsed();
        if waited < Duration::from_millis(32) {
            return;
        }
        let (_, change) = self.preparing.take().unwrap();
        let started = Instant::now();
        // Every change queued meanwhile joins this one. After a failed
        // batch its changes go one at a time again, so that one bad
        // component fails only its own mount.
        let mut changes = vec![change];
        if self.mount_singly > 0 {
            self.mount_singly -= 1;
        } else {
            for change in self.mount_queue.drain(..).collect::<Vec<_>>() {
                if !change.announced {
                    self.send(
                        change.reply.owner,
                        json!({"event":"ui.preparing","data":{"phase":"preparing","slot":slot_of(change.key)}}),
                    );
                }
                changes.push(change);
            }
        }
        // A mount of a plugin stopped since is dropped, as before; a tab
        // replayed from a recording belongs to a plugin in standby, which has
        // no process (dropped when it is no longer held either).
        changes.retain(|change| {
            let owner = change.reply.owner;
            match &change.contribution {
                None => true,
                Some(mount) if mount.recorded => {
                    self.plugins.contains_key(&owner) || self.on_demand.held(owner).is_some()
                }
                Some(_) => self.plugins.contains_key(&owner),
            }
        });
        if changes.is_empty() {
            return;
        }
        let mut next = self.mounts.clone();
        for change in &changes {
            let owner = change.reply.owner;
            if let Some(mount) = &change.contribution {
                // Mounted after all: an earlier refusal no longer applies.
                self.clear_warning(owner, "slot-occupied");
                next.insert(change.key, mount.clone());
            } else {
                next.remove(&change.key);
            }
        }
        let batch = changes.iter().map(|c| c.key).collect::<Vec<_>>();
        let result = self.apply_mounts(next, &batch);
        self.log_prepared(changes.len(), started, result.is_ok());
        if result.is_err() && changes.len() > 1 {
            // Again one at a time, from the next tick on: the first, then
            // the rest in their order.
            self.mount_singly = changes.len();
            let mut changes = changes.into_iter();
            let first = changes.next().expect("more than one change");
            for mut change in changes.rev() {
                change.announced = true;
                self.mount_queue.push_front(change);
            }
            let due = Instant::now()
                .checked_sub(Duration::from_millis(32))
                .unwrap_or_else(Instant::now);
            self.preparing = Some((due, first));
            return;
        }
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        for change in changes {
            let owner = change.reply.owner;
            // A panel kept over its plugin's restart, mounted again (from
            // its compiled file when that did not change) or removed: its
            // restart line goes (`panel_restart.rs`).
            if result.is_ok() && self.restarts.waiting(change.key) {
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
                self.panel_back(change.key);
            }
            if !change.contribution.as_ref().is_some_and(|m| m.recorded) {
                self.send(owner,json!({"event":"ui.preparing","data":{"phase":if result.is_ok(){"ready"}else{"failed"},"elapsedMs":elapsed,"slot":slot_of(change.key)}}));
            }
            if change.reply.id == 0 {
                if let Err(error) = &result {
                    self.broadcast(
                        "host.error",
                        json!({"code":"ui-owner-cleanup","owner":owner,"message":error.message}),
                    );
                }
            } else {
                self.respond(change.reply, result.clone());
            }
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
    /// in the background compiles nothing, and one that turns out not to
    /// compile when it is first shown is reported to its plugin then
    /// (`show_panel`). The policy, the panel keys and the remembered state
    /// follow.
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
                Some(column) => {
                    plans[column.index()].active == Some(mount.spec.owner)
                        && self.column_wanted(column)
                        && !self.awaits_remembered(column, Some(mount.spec.owner), &next)
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
        self.policy.activity(self.now_ms());
        {
            let mut input = self.input.borrow_mut();
            input.bottom = next
                .values()
                .find(|m| m.spec.slot == composition::Slot::Bottom)
                .map(|m| m.plugin);
            // The panel keys' plugins: their commands take the panel's keys.
            input.sidebar = self.policy.sidebar_active().map(panel_plugin);
            input.adjust = self
                .policy
                .active(window_state::Slot::Adjust)
                .map(panel_plugin);
        }
        self.mounts = next;
        self.note_panel_plugins();
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
    /// During start-up, a column whose remembered current tab has not
    /// mounted yet (plugins start one after another) waits for it, instead
    /// of compiling and showing the tab that is current meanwhile (a flash
    /// of another tab, and a compile for nothing): the single shell waited
    /// for every start-up mount. Once start-up settled (`startup_panels`)
    /// the current tab shows, whatever it is (the remembered plugin may be
    /// gone).
    fn awaits_remembered(
        &self,
        column: Column,
        active: Option<u64>,
        mounts: &BTreeMap<u64, Mount>,
    ) -> bool {
        if self.startup_panels {
            return false;
        }
        let Some(remembered) = self.column_memory(column).active else {
            return false;
        };
        active
            .and_then(|active| mounts.get(&active))
            .is_none_or(|mount| mount.manifest != remembered)
            && !mounts
                .values()
                .any(|mount| mount.spec.slot == column.slot() && mount.manifest == remembered)
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
                self.send(panel_plugin(key), ui_error(&refusal));
                matches!(
                    self.panels.show(window, generation, slot, None, &events),
                    Ok(Shown::Changed { .. })
                )
            }
        }
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
            let wanted =
                self.column_wanted(column) && !self.awaits_remembered(column, active, &self.mounts);
            // Not wanted shown (or waiting for its remembered tab): what it
            // showed stays (nothing compiles), unless that is no longer its
            // current tab.
            let key = active.filter(|&a| wanted || self.panels.shown(generation, slot) == Some(a));
            if self.show_panel(&shell, generation, slot, key) {
                changed = true;
                self.panels.evict(slot);
            }
            if column == Column::Sidebar
                && let Some((window, floating)) = &floating
                && self.show_panel(window, *floating, slot, active)
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
            self.send(panel_plugin(owner), ui_error(&RpcError::new("ui", message)));
        }
    }

    /// Initial properties `refuse_initial` left out: logged and reported
    /// to the plugin (`ui.error`, its `onError`).
    pub(crate) fn report_refused(&mut self, owner: u64, refused: Vec<RpcError>) {
        for error in refused {
            diag!("initial property {owner}: {}", error.message);
            self.send(owner, ui_error(&error));
        }
    }
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
                    self.send(owner, ui_error(&error));
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
            if self.restarts.waiting(event.owner) {
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
            match event.args {
                Ok(args)=>self.send(owner,json!({"event":"ui.callback","data":{"view":"$mount","slot":slot,"callback":event.callback,"args":args}})),
                Err(error)=>self.send(owner,json!({"event":"ui.error","data":{"code":"ui","slot":slot,"message":format!("callback {}: {error}",event.callback)}})),
            }
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
