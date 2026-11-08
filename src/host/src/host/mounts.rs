//! Mounted panels: preparing the shell for mount changes, their values
//! and callbacks.
use super::*;

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
                        json!({"event":"ui.preparing","data":{"phase":"preparing"}}),
                    );
                }
                self.preparing = Some((Instant::now(), change));
                return;
            }
            // The current tab's panel is not in the shell (a running
            // plugin's tab beyond the front tabs, sidebar-tabs.md): the shell
            // is compiled again with it.
            if std::mem::take(&mut self.shell_refresh) && self.shell.is_some() {
                let active = self.policy.sidebar_active();
                let started = Instant::now();
                let result = self.apply_mounts(self.mounts.clone(), &[]);
                if let Err(error) = &result {
                    diag!("sidebar tab {active:?}: {}", error.message);
                    self.refresh_failed = active;
                }
                self.log_prepared(0, started, result.is_ok());
            }
            return;
        }
        let waited = self.preparing.as_ref().unwrap().0.elapsed();
        if waited < Duration::from_millis(32) {
            return;
        }
        // At start-up, a built-in that will mount its panel and has not
        // asked yet is waited for (briefly), to compile the shell once.
        if waited < STARTUP_MOUNT_GATHER && self.builtin_mount_expected() {
            return;
        }
        let (_, change) = self.preparing.take().unwrap();
        let started = Instant::now();
        // Every change queued meanwhile joins this one: one compile of the
        // shell instead of one per mount (start-up mounts arrive together).
        // After a failed batch its changes go one at a time again, so that
        // one bad component fails only its own mount.
        let mut changes = vec![change];
        if self.mount_singly > 0 {
            self.mount_singly -= 1;
        } else {
            for change in self.mount_queue.drain(..).collect::<Vec<_>>() {
                if !change.announced {
                    self.send(
                        change.reply.owner,
                        json!({"event":"ui.preparing","data":{"phase":"preparing"}}),
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
                next.insert(owner, mount.clone());
            } else {
                next.remove(&owner);
            }
        }
        let batch = changes.iter().map(|c| c.reply.owner).collect::<Vec<_>>();
        let result = self.apply_mounts(next, &batch);
        self.log_prepared(changes.len(), started, result.is_ok());
        if result.is_err() && changes.len() > 1 {
            // Again one at a time, from the next tick on (each still one
            // compile per tick): the first, then the rest in their order.
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
            if !change.contribution.as_ref().is_some_and(|m| m.recorded) {
                self.send(owner,json!({"event":"ui.preparing","data":{"phase":if result.is_ok(){"ready"}else{"failed"},"elapsedMs":elapsed}}));
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
                self.shell_specs.len(),
                started.elapsed().as_secs_f64() * 1000.,
                if ok { "ready" } else { "failed" }
            );
        }
    }
    /// Make `next` the mounted panels (sidebar-tabs.md); `batch`: the owners
    /// whose mount or unmount this applies (empty for a host-initiated
    /// compile). The shell is compiled again only when what it compiles
    /// changes: the bottom bar, the backdrop, the sidebar's compiled tabs
    /// (`sidebar_tabs::compiled`: the front tabs) or whether it draws the tab
    /// strip. Otherwise only the strip's titles change, and a batch owner's
    /// compiled panel gets its new values. A sidebar panel that joins the
    /// compiled set without a mount of its own in the batch (a switch to its
    /// tab) and breaks the compile is left out and reported to its plugin,
    /// so it never fails the other plugins' mounts. The policy, the panel
    /// keys and the remembered state follow either way.
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
        let order = self.tab_order(&next);
        let slot = self.sidebar_slot();
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
        // The tab that will be current, so the first frame shows it and the
        // shell compiles once: the policy's after the leaving tabs went (the
        // next one takes over a removed current tab). A placeholder that
        // would take over that way is passed over for a running plugin's
        // tab, so that removing a tab never wakes a neighbour unasked.
        let mut planned = self.policy.clone();
        for owner in &leaving {
            planned.remove_owner(*owner);
        }
        let current_left = self
            .policy
            .sidebar_active()
            .is_some_and(|owner| leaving.contains(&owner));
        let recorded = |owner: u64| next.get(&owner).is_some_and(|m| m.recorded);
        let active =
            sidebar_tabs::next_active(&order, planned.sidebar_active(), current_left, recorded)
                .or_else(|| {
                    order.iter().copied().find(|o| {
                        next.get(o).map(|m| m.manifest.as_str()) == slot.active.as_deref()
                    })
                })
                .or_else(|| order.first().copied());
        let tabbed = order.len() > 1;
        let titles = ui_values::from_json(&json!(
            order
                .iter()
                .filter_map(|owner| next.get(owner).map(|m| m.title.clone()))
                .collect::<Vec<_>>()
        ))
        .map_err(|error| RpcError::new("ui-prepare", error.to_string()))?;
        let mut recompiled = false;
        let mut shown;
        loop {
            let failed = self.tabs_failed.clone();
            let compiled_tabs = sidebar_tabs::compiled(
                &order,
                active,
                recorded,
                |owner| failed.contains(&owner),
                &self.shell_compiled,
            );
            shown = next
                .values()
                .filter(|m| m.spec.slot != composition::Slot::Sidebar)
                .chain(compiled_tabs.iter().filter_map(|owner| next.get(owner)))
                .cloned()
                .collect::<Vec<_>>();
            let specs = shown.iter().map(|m| m.spec.clone()).collect::<Vec<_>>();
            if self.shell.is_some() && specs == self.shell_specs && tabbed == self.shell_tabbed {
                break;
            }
            let compiled = compiled_tabs
                .iter()
                .map(|&owner| (owner, owner))
                .collect::<BTreeMap<_, _>>();
            let active_index = active
                .and_then(|a| order.iter().position(|&o| o == a))
                .unwrap_or(0);
            match self.compile_shell(
                &shown,
                tabbed,
                titles.clone(),
                active_index,
                sidebar_tabs::shown(active, &compiled),
            ) {
                Ok(failures) => {
                    self.report_value_failures(failures);
                    self.shell_specs = specs;
                    self.shell_tabbed = tabbed;
                    self.shell_compiled = compiled;
                    recompiled = true;
                    break;
                }
                Err(error) => {
                    // Sidebar panels this compile added without a mount of
                    // their own in the batch: left out, once.
                    let added = sidebar_tabs::blamed(&compiled_tabs, batch, &self.shell_compiled);
                    if added.is_empty() || added.iter().any(|o| self.tabs_failed.contains(o)) {
                        return Err(RpcError::new("ui-prepare", error.to_string()));
                    }
                    for owner in added {
                        diag!("sidebar tab {owner}: its panel does not compile: {error:#}");
                        self.tabs_failed.insert(owner);
                        let refusal = RpcError::new("ui-prepare", format!("ui-prepare: {error:#}"));
                        self.send(owner, ui_error(&refusal));
                    }
                }
            }
        }
        if recompiled {
            for (id, mount) in &next {
                let Some(slot) = policy_slot(mount.spec.slot) else {
                    continue;
                };
                if let Err(error) = self
                    .policy
                    .set_locks(slot, *id, window_state::Locks::default())
                {
                    diag!("mount policy reset: {error}");
                }
            }
        } else if let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) {
            // The same shell: the strip's titles (a tab came or went), and
            // the new values of a batch owner's compiled panel (an unmount
            // and a mount of the same panel in one batch).
            let error = |error: anyhow::Error| RpcError::new("ui-prepare", error.to_string());
            shell
                .set_property("sidebar-titles", titles)
                .map_err(|e| error(e.into()))?;
            for mount in shown
                .iter()
                .filter(|m| m.spec.slot == composition::Slot::Sidebar)
            {
                composition::set_tabbed(&shell, &mount.spec, tabbed).map_err(error)?;
            }
            let rewritten = shown
                .iter()
                .filter(|m| batch.contains(&m.spec.owner))
                .cloned()
                .collect::<Vec<_>>();
            let mut failures = Vec::new();
            for mount in &rewritten {
                for (name, value) in self.mount_values(mount) {
                    if let Err(error) =
                        composition::set_global_value(&shell, &mount.spec, &name, value)
                    {
                        failures.push((mount.spec.owner, format!("{name}: {error:#}")));
                    }
                }
                if let Some(slot) = policy_slot(mount.spec.slot)
                    && let Err(error) = self.policy.set_locks(
                        slot,
                        mount.spec.owner,
                        window_state::Locks::default(),
                    )
                {
                    diag!("mount policy reset: {error}");
                }
            }
            self.report_value_failures(failures);
        }
        for old in &leaving {
            self.policy.remove_owner(*old);
            if !next.contains_key(old) {
                self.images.release_view(*old, "$mount");
            }
        }
        let attach = next
            .values()
            .filter(|m| m.spec.slot != composition::Slot::Sidebar)
            .chain(order.iter().filter_map(|owner| next.get(owner)))
            .filter(|m| !self.mounts.contains_key(&m.spec.owner) || leaving.contains(&m.spec.owner))
            .map(|m| (m.spec.owner, m.spec.slot, m.visible, m.manifest.clone()))
            .collect::<Vec<_>>();
        for (id, mount_slot, mount_visible, manifest) in attach {
            let Some(policy) = policy_slot(mount_slot) else {
                continue;
            };
            let (visible, detached) = match policy {
                window_state::Slot::Sidebar => (slot.visible, slot.detached),
                window_state::Slot::Bottom => (mount_visible, false),
            };
            if let Err(error) = self.policy.attach(policy, id, visible, detached) {
                diag!("mount policy attach: {error}");
            }
            // The tab that was current at the last run is current again.
            if policy == window_state::Slot::Sidebar && slot.active.as_deref() == Some(&manifest) {
                let _ = self.policy.set_active(id);
            }
        }
        // The tab chosen above, when the current one left.
        if current_left && let Some(active) = active {
            let _ = self.policy.set_active(active);
        }
        self.policy.order_sidebar(&order);
        self.policy.activity(self.now_ms());
        {
            let mut input = self.input.borrow_mut();
            input.bottom = next
                .values()
                .find(|m| m.spec.slot == composition::Slot::Bottom)
                .map(|m| m.spec.owner);
            input.sidebar = self.policy.sidebar_active();
        }
        self.mounts = next;
        self.shell_tabs = order;
        self.remember_sidebar(false);
        Ok(
            json!({"status":"ready","shellGeneration":self.shell_generation,"prepareMs":started.elapsed().as_secs_f64()*1000.}),
        )
    }
    /// Values a new instance could not take (a property the panel does not
    /// declare, a value of the wrong type): logged and reported to their
    /// plugin (`ui.error`, its `onError`), as `mount_values` does; the panel
    /// is drawn with its defaults for them.
    pub(crate) fn report_value_failures(&mut self, failures: Vec<(u64, String)>) {
        for (owner, message) in failures {
            diag!("mount property {owner}/{message}");
            self.send(owner, ui_error(&RpcError::new("ui", message)));
        }
    }
    /// Compile the shell with `shown` (every panel it draws: the bottom bar,
    /// the backdrop, the sidebar's compiled tabs) inside the main window,
    /// with their values, and make it current. The values a panel could not
    /// take are returned (owner, what), for `report_value_failures`: one
    /// bad value never fails the compile.
    fn compile_shell(
        &mut self,
        shown: &[Mount],
        tabbed: bool,
        titles: SValue,
        active_index: usize,
        shown_owner: i64,
    ) -> Result<Vec<(u64, String)>> {
        let failures = RefCell::new(Vec::new());
        self.next_shell_generation += 1;
        let candidate_generation = self.next_shell_generation;
        let native_before = platform::capture_windows().ok();
        let values: Vec<_> = shown.iter().map(|mount| self.mount_values(mount)).collect();
        let slot = self.sidebar_slot();
        let sidebar = self
            .policy
            .sidebar_state()
            .unwrap_or((slot.visible, slot.detached));
        // The main window stays: the new shell's tree replaces the old one
        // inside it (`view::replace_shell`), with no hide/show, move or resize.
        let reuse = self.shell.is_some();
        let build =
            |window: Option<&slint::Window>| -> Result<slint_interpreter::ComponentInstance> {
                let specs = shown.iter().map(|m| m.spec.clone()).collect::<Vec<_>>();
                let shell = composition::compile_json_in(
                    &specs,
                    tabbed,
                    self.mount_events.clone(),
                    candidate_generation,
                    window,
                )?;
                for (mount, values) in shown.iter().zip(values) {
                    for (name, value) in values {
                        if let Err(error) =
                            composition::set_global_value(&shell, &mount.spec, &name, value)
                        {
                            failures
                                .borrow_mut()
                                .push((mount.spec.owner, format!("{name}: {error:#}")));
                        }
                    }
                    let visible = match mount.spec.slot {
                        composition::Slot::Sidebar => {
                            composition::set_tabbed(&shell, &mount.spec, tabbed)?;
                            sidebar.0 && !sidebar.1
                        }
                        _ => mount.visible && !mount.detached,
                    };
                    if let Some(property) = mount.spec.slot.visible_property() {
                        shell.set_property(property, visible.into())?;
                    }
                }
                shell.set_property("sidebar-titles", titles)?;
                shell.set_property("sidebar-active", SValue::Number(active_index as f64))?;
                shell.set_property("sidebar-shown", SValue::Number(shown_owner as f64))?;
                self.bind_shell(&shell, candidate_generation)?;
                self.bind_tabs(&shell, candidate_generation)?;
                if window.is_some() {
                    // `replace_shell` shows it in the window, which keeps its
                    // rendering notifier (`retarget_shell`).
                    return Ok(shell);
                }
                let texture = self.attach_gpu(&shell);
                shell.show()?;
                // Only a shown candidate replaces the main window's texture.
                self.video_texture.replace(texture);
                platform::shown(&shell);
                frame_pacing::follow(&shell);
                Ok(shell)
            };
        let shell = view::replace_shell(self.shell.as_ref(), view::focus_video, build)?;
        // The window's one event hook now reports for this shell (not
        // set while building: a failed candidate leaves the old one).
        self.bind_window_events(&shell, candidate_generation);
        self.theme_view(&shell);
        // Emulator key capture stays: the window was neither hidden
        // nor unfocused, and the new tree gives the video area the
        // focus (its `init`); each key is still checked against the
        // new shell's `canvas-focused`. A capture revoked here used
        // to end at every plugin install or uninstall.
        if reuse {
            self.retarget_shell(&shell);
        } else {
            self.native_before = native_before.clone();
            self.native_window = native_before
                .as_ref()
                .and_then(|before| platform::associate_new(before).ok());
            self.video_target.replace(shell.as_weak());
        }
        // Not hidden: it shared the window. Dropping it frees its tree.
        drop(self.shell.replace(shell));
        // A message on screen stays, with its buttons; its remaining time
        // and hold are the host's (`osd_until`), unchanged.
        if self.osd_shown.visible {
            self.osd_debug(&OsdEvent::Kept);
        }
        self.write_osd();
        // And the plugins' information blocks (`osd.panel`), lost likewise.
        self.show_info_panels();
        if self.menu_generation == self.shell_generation {
            self.menu_open = false;
            self.menu_mode = MenuMode::Main;
        }
        self.shell_generation = candidate_generation;
        Ok(failures.into_inner())
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
        let owner = mount.spec.owner;
        let mut values = Vec::new();
        for (name, value) in &mount.properties {
            match self.prop_value(owner, "$mount", name, value) {
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
            if !self.window_generation_live(event.shell_generation)
                || !self.mounts.contains_key(&event.owner)
            {
                continue;
            }
            match event.args {
                Ok(args)=>self.send(event.owner,json!({"event":"ui.callback","data":{"view":"$mount","callback":event.callback,"args":args}})),
                Err(error)=>self.send(event.owner,json!({"event":"ui.error","data":{"code":"ui","message":format!("callback {}: {error}",event.callback)}})),
            }
        }
    }
}
