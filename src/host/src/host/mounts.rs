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
        // A mount of a plugin stopped since is dropped, as before.
        changes.retain(|change| {
            change.contribution.is_none() || self.plugins.contains_key(&change.reply.owner)
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
        self.next_shell_generation += 1;
        let candidate_generation = self.next_shell_generation;
        let native_before = platform::capture_windows().ok();
        // The sidebar's panels in tab order (after the other slots' panels).
        let order = self.tab_order(&next);
        let shown = next
            .values()
            .filter(|m| m.spec.slot != composition::Slot::Sidebar)
            .chain(order.iter().filter_map(|owner| next.get(owner)))
            .cloned()
            .collect::<Vec<_>>();
        let values: Vec<_> = shown.iter().map(|mount| self.mount_values(mount)).collect();
        let slot = self.sidebar_slot();
        let sidebar = self
            .policy
            .sidebar_state()
            .unwrap_or((slot.visible, slot.detached));
        let titles = ui_values::from_json(&json!(
            order
                .iter()
                .filter_map(|owner| next.get(owner).map(|m| m.title.clone()))
                .collect::<Vec<_>>()
        ));
        // The main window stays: the new shell's tree replaces the old one
        // inside it (`view::replace_shell`), with no hide/show, move or resize.
        let reuse = self.shell.is_some();
        let build =
            |window: Option<&slint::Window>| -> Result<slint_interpreter::ComponentInstance> {
                let specs = shown.iter().map(|m| m.spec.clone()).collect::<Vec<_>>();
                let shell = composition::compile_json_in(
                    &specs,
                    self.mount_events.clone(),
                    candidate_generation,
                    window,
                )?;
                for (mount, values) in shown.iter().zip(values) {
                    for (name, value) in values {
                        composition::set_global_value(&shell, &mount.spec, &name, value)?;
                    }
                    let visible = match mount.spec.slot {
                        composition::Slot::Sidebar => {
                            composition::set_tabbed(&shell, &mount.spec, order.len() > 1)?;
                            sidebar.0 && !sidebar.1
                        }
                        _ => mount.visible && !mount.detached,
                    };
                    if let Some(property) = mount.spec.slot.visible_property() {
                        shell.set_property(property, visible.into())?;
                    }
                }
                shell.set_property("sidebar-titles", titles?)?;
                // The tab that will be current, so the first frame shows it.
                let active = order
                    .iter()
                    .position(|&o| Some(o) == self.policy.sidebar_active())
                    .or_else(|| {
                        order.iter().position(|o| {
                            next.get(o).map(|m| m.manifest.as_str()) == slot.active.as_deref()
                        })
                    })
                    .unwrap_or(0);
                shell.set_property("sidebar-active", SValue::Number(active as f64))?;
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
        let result = match view::replace_shell(self.shell.as_ref(), build) {
            Ok(shell) => {
                self.theme_view(&shell);
                #[cfg(feature = "producer-audio")]
                self.physical_revoke(producer_input::RevokeReason::WindowHidden);
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
                let old_owners = self.mounts.keys().copied().collect::<Vec<_>>();
                for old in old_owners {
                    if !next.contains_key(&old) {
                        self.policy.remove_owner(old);
                        self.images.release_view(old, "$mount");
                    }
                }
                for mount in &shown {
                    let id = mount.spec.owner;
                    if self.mounts.contains_key(&id) {
                        continue;
                    }
                    let Some(policy) = policy_slot(mount.spec.slot) else {
                        continue;
                    };
                    let (visible, detached) = match policy {
                        window_state::Slot::Sidebar => (slot.visible, slot.detached),
                        window_state::Slot::Bottom => (mount.visible, false),
                    };
                    if let Err(error) = self.policy.attach(policy, id, visible, detached) {
                        diag!("mount policy attach: {error}");
                    }
                    // The tab that was current at the last run is current again.
                    if policy == window_state::Slot::Sidebar
                        && slot.active.as_deref() == Some(mount.manifest.as_str())
                    {
                        let _ = self.policy.set_active(id);
                    }
                }
                self.policy.order_sidebar(&order);
                for (id, mount) in &next {
                    let Some(slot) = policy_slot(mount.spec.slot) else {
                        continue;
                    };
                    if let Err(error) =
                        self.policy
                            .set_locks(slot, *id, window_state::Locks::default())
                    {
                        diag!("mount policy reset: {error}");
                    }
                }
                self.policy.activity(self.now_ms());
                {
                    let mut input = self.input.borrow_mut();
                    input.bottom = next
                        .values()
                        .find(|m| m.spec.slot == composition::Slot::Bottom)
                        .map(|m| m.spec.owner);
                    input.sidebar = self.policy.sidebar_active();
                }
                if self.menu_generation == self.shell_generation {
                    self.menu_open = false;
                    self.menu_mode = MenuMode::Main;
                }
                self.mounts = next;
                self.shell_generation = candidate_generation;
                self.shell_tabs = order;
                self.remember_sidebar(false);
                Ok(
                    json!({"status":"ready","shellGeneration":candidate_generation,"prepareMs":started.elapsed().as_secs_f64()*1000.}),
                )
            }
            Err(error) => Err(RpcError::new("ui-prepare", error.to_string())),
        };
        if self.frame_log.is_some() {
            diag!(
                "vestra shell: {} change(s), {} panel(s), prepared in {:.1} ms ({})",
                changes.len(),
                self.mounts.len(),
                started.elapsed().as_secs_f64() * 1000.,
                if result.is_ok() { "ready" } else { "failed" }
            );
        }
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
            self.send(owner,json!({"event":"ui.preparing","data":{"phase":if result.is_ok(){"ready"}else{"failed"},"elapsedMs":elapsed}}));
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
