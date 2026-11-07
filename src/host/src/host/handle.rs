//! `Host::handle`: one event of the UI thread's loop.
use super::*;

impl Host {
    /// The prepared rows of the request `reply` answers (`Event::Prepared`),
    /// taken: each request has one value.
    pub(crate) fn take_prepared(
        &mut self,
        reply: &Reply,
    ) -> Option<Box<crate::ui::ui_values::Prepared>> {
        match &self.prepared {
            Some((owner, id, _)) if *owner == reply.owner && *id == reply.id => {
                self.prepared.take().map(|(_, _, prepared)| prepared)
            }
            _ => None,
        }
    }
    pub(crate) fn handle(&mut self, event: Event) {
        // What a process said before the Host has it (its spawn on the
        // spawn thread is not answered yet) waits for it (`worker.rs`).
        let from_process = super::supervisor::worker::from_process(&event);
        if let Some(owner) = from_process
            && self.spawning(owner)
        {
            self.hold_spawning_event(owner, event);
            return;
        }
        match event {
            Event::Prepared(owner, message, prepared) => {
                self.prepared = message["id"].as_u64().map(|id| (owner, id, prepared));
                self.handle(Event::Request(owner, message));
                self.prepared = None;
            }
            Event::Request(owner, message) => {
                if !self.plugins.contains_key(&owner) {
                    return;
                }
                // On-demand start: a plugin that asks for something is in use.
                let now = self.now_ms();
                self.supervisor.touch(Pid(owner), now);
                if let Some(event) = message["event"].as_str() {
                    diag!("runtime {owner} {event}: {}", message["data"]);
                    let now = self.now_ms();
                    let result = match event {
                        "runtime.ready" => self.supervisor.activation_ready(Pid(owner), now),
                        "runtime.failed" => self.supervisor.activation_failed(
                            Pid(owner),
                            message["data"].to_string().chars().take(512).collect(),
                            now,
                        ),
                        _ => Ok(()),
                    };
                    if let Err(error) = result {
                        diag!("activation runtime event: {error}");
                    }
                    self.activation_tick();
                    if event == "runtime.ready" && self.plugins.contains_key(&owner) {
                        self.unit_ready(owner);
                    }
                    return;
                }
                match serde_json::from_value::<Request>(message) {
                    Ok(request) => {
                        let reply = Reply {
                            owner,
                            id: request.id,
                        };
                        if matches!(request.method.as_str(), "ui.open" | "ui.mount") {
                            let now = self.now_ms();
                            if matches!(
                                self.activation_status(owner, now),
                                Ok(activation::Status::Pending { .. })
                            ) {
                                if let Err(error) =
                                    self.supervisor
                                        .activation_ui_begin(Pid(owner), reply.id, now)
                                {
                                    self.respond(
                                        reply,
                                        Err(RpcError::new("activation", error.to_string())),
                                    );
                                    return;
                                }
                                self.supervisor.activation_ui_wait(Pid(owner), reply.id);
                                self.ui_begun(owner);
                            }
                        }
                        // Built-in plugins claim panel slots, the sequence and resolvers first (slot_claims).
                        if matches!(
                            request.method.as_str(),
                            "ui.mount"
                                | "ui.unmount"
                                | "sequence.set"
                                | "sourceResolvers.register"
                                | "sourceResolvers.unregister"
                        ) && !self.headless
                        {
                            let builtin = self.is_builtin(owner);
                            let starting = self.builtins_starting();
                            if self.held_mounts.holds(owner, builtin, starting) {
                                self.held_mounts
                                    .hold(owner, (reply, request.method, request.params));
                                return;
                            }
                        }
                        self.answer(reply, &request.method, &request.params);
                    }
                    Err(error) => {
                        diag!("malformed request: {error}");
                        self.stop_failed(owner);
                    }
                }
            }
            #[cfg(feature = "producer-audio")]
            Event::PhysicalInput(generation, params) => {
                if generation == self.shell_generation {
                    self.physical_input(params);
                }
            }
            Event::WindowInput(generation, kind) => {
                if generation != self.shell_generation {
                    return;
                }
                let _ = kind;
                // Activity keeps the pointer and the popped-out sidebar; it
                // never reveals the fullscreen bar (FS-BAR-1).
                self.policy.activity(self.now_ms());
            }
            Event::MenuAction(generation, action) => {
                if !self.window_generation_live(generation)
                    || (!matches!(action, MenuAction::Reset) && generation != self.menu_generation)
                {
                    return;
                }
                self.policy.activity(self.now_ms());
                if debug_menu()
                    && let MenuAction::Invoke(token) = &action
                {
                    diag!("menu: invoke {token} open={:?}", self.menu_path);
                }
                match action {
                    MenuAction::Reset => {
                        #[cfg(feature = "producer-audio")]
                        self.physical_revoke(producer_input::RevokeReason::Menu);
                        self.open_menu(generation, MenuMode::Main);
                    }
                    MenuAction::Back => {
                        let mut path = self.menu_path.clone();
                        path.pop();
                        self.set_menu_path(path);
                    }
                    MenuAction::Dismiss => self.close_menu(),
                    MenuAction::Palette(action) => self.palette_action(action),
                    MenuAction::Hover(level, token, now) => {
                        if debug_menu() {
                            diag!(
                                "menu: hover level={level} token={token} now={now} open={:?} t={}",
                                self.menu_path,
                                self.now_ms()
                            );
                        }
                        let Some(path) = menu::hover_target(&self.menu_path, level, &token) else {
                            return;
                        };
                        if now {
                            self.set_menu_path(path);
                        } else {
                            // The open submenu beside the hovered level.
                            let submenu = (self.menu_path.len() > level)
                                .then(|| self.menu_panels.get(level + 1).copied())
                                .flatten();
                            let pointer = self.menu_pointer();
                            let now = self.now_ms();
                            if let Some(path) =
                                self.menu_aim
                                    .hover(&self.menu_path, path, pointer, submenu, now)
                            {
                                self.set_menu_path(path);
                            }
                        }
                    }
                    MenuAction::Scroll(level, offset) => {
                        if level <= self.menu_path.len() && offset.is_finite() {
                            self.menu_scroll.resize(level + 1, 0.);
                            self.menu_scroll[level] = offset;
                            let result = self.refresh_menu();
                            self.report_menu(result);
                        }
                    }
                    MenuAction::Invoke(token) => match menu::parse_token(&token) {
                        Some(menu::Token::Group(path)) => self.set_menu_path(path),
                        Some(menu::Token::Command { owner, id }) => {
                            self.close_menu();
                            if let Err(error) = self.execute_command(owner, &id) {
                                diag!("menu execute: {:?}", error);
                            }
                        }
                        Some(menu::Token::Tab(owner)) => {
                            self.close_menu();
                            if self.policy.set_active(owner).is_ok() {
                                let column = self
                                    .mounts
                                    .get(&owner)
                                    .and_then(|m| sidebar_tabs::Column::of(m.spec.slot))
                                    .unwrap_or(sidebar_tabs::Column::Sidebar);
                                self.remember_column(column, true);
                                self.window_tick();
                            }
                        }
                        None => (),
                    },
                }
            }
            Event::ActionDone(reply, action, result) => {
                input_trace::mark(&format!("host loop: {action} done"));
                self.action_done(reply, &action, result)
            }
            Event::Reply(reply, mut result) => {
                if self.source_state_replies.remove(&(reply.owner, reply.id))
                    && let Ok(state) = &mut result
                {
                    self.patch_source(state);
                }
                self.respond(reply, result)
            }
            Event::Broadcast(event, mut data) => {
                if event == "source.state" {
                    self.patch_source(&mut data);
                    let previous = std::mem::replace(&mut self.last_source, data.clone());
                    // What a restart after a lost GPU device reopens.
                    *LAST_SOURCE
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = data.clone();
                    if previous["phase"] != data["phase"] {
                        input_trace::mark(&format!(
                            "host loop: source.state phase={}",
                            data["phase"]
                        ));
                    }
                    if previous["paused"] != data["paused"] {
                        input_trace::mark(&format!(
                            "host loop: source.state paused={}",
                            data["paused"]
                        ));
                    }
                    self.timeline_source(&previous, &data);
                    self.backdrop_source();
                    self.update_title();
                    self.core_action_states();
                }
                if event == "resolvers.fileTypes" {
                    self.file_types = data["fileTypes"].clone();
                }
                self.broadcast(&event, data)
            }
            Event::Binding(generation, owner, id) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                input_trace::mark(&format!("host loop: binding → {owner}/{id}"));
                self.policy.activity(self.now_ms());
                match self.commands.resolve_execution(owner, &id) {
                    // A key never fails silently: the command's reason shows.
                    Err(commands::CommandError::Disabled {
                        reason: Some(reason),
                    }) => self.show_osd(&reason),
                    _ => {
                        if let Err(error) = self.execute_command(owner, &id) {
                            diag!("binding execute: {error:?}");
                        }
                    }
                }
            }
            Event::SidebarResized(generation, width) => {
                if generation != self.shell_generation {
                    return;
                }
                self.policy.set_sidebar_width(width);
                self.ui_state.state.sidebar = Some(ui_state::Sidebar { width });
                self.ui_state.touch(self.now_ms());
            }
            Event::BarButton(generation, token) => self.bar_click(generation, &token),
            Event::BarMore(generation, x, y) => self.bar_more(generation, x, y),
            Event::OwnerResources(owner, answer) => self.resources_answer(owner, &answer),
            Event::PipReturn(generation) => {
                if generation == self.shell_generation {
                    self.pip.restore();
                }
            }
            Event::FloatingClosed(generation) => {
                if self
                    .floating
                    .as_ref()
                    .is_some_and(|f| f.generation == generation)
                {
                    self.close_sidebar();
                    self.close_floating();
                }
            }
            Event::Tab(generation, column, index) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                // The index is in that window's tab order.
                let tabs = match &self.floating {
                    Some(floating)
                        if floating.generation == generation
                            && column == sidebar_tabs::Column::Sidebar =>
                    {
                        &floating.owners
                    }
                    _ => &self.columns[column.index()].tabs,
                };
                if let Some(&owner) = tabs.get(index)
                    && self.policy.set_active(owner).is_ok()
                {
                    self.columns[column.index()].tab_clicked = true;
                    self.remember_column(column, true);
                    self.window_tick();
                }
            }
            Event::TabOverflow(generation, column, x, y) => {
                if !self.window_generation_live(generation) || !x.is_finite() || !y.is_finite() {
                    return;
                }
                if let Some(window) = self
                    .window_for(generation)
                    .map(ComponentHandle::clone_strong)
                {
                    let _ = window.set_property("menu-origin-x", x.into());
                    let _ = window.set_property("menu-origin-y", y.into());
                }
                self.open_menu(generation, MenuMode::Tabs(column));
            }
            Event::AdjustResized(generation, width) => {
                if generation != self.shell_generation {
                    return;
                }
                self.policy.set_adjust_width(width);
                let mut slot = self.adjust_slot();
                slot.width = self.policy.adjust_user_width();
                self.ui_state.state.adjust_slot = Some(slot);
                self.ui_state.touch(self.now_ms());
            }
            Event::AdjustDismiss(generation) => {
                if generation == self.shell_generation {
                    self.set_adjust_visible(false);
                }
            }
            Event::SidebarDetach(generation) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                if let (Some(owner), Some((_, detached))) =
                    (self.policy.sidebar_active(), self.policy.sidebar_state())
                    && self.policy.set_detached(owner, !detached).is_ok()
                {
                    self.remember_sidebar(false);
                    self.window_tick();
                }
            }
            Event::PluginMessage(owner, event, data) => {
                self.send(owner, json!({"event":event,"data":data}))
            }
            Event::MainClosed => self.host_command("quit"),
            Event::TitleStrip(generation, action) => {
                if generation == self.shell_generation {
                    self.title_strip_action(&action);
                }
            }
            Event::OsdAction(generation, index) => {
                if generation == self.shell_generation {
                    self.osd_action(index);
                }
            }
            Event::ViewClosed(owner, id) => {
                if let Some(view) = self.views.remove(&(owner, id.clone())) {
                    let _ = view.instance.hide();
                    self.images.release_view(owner, &format!("w:{id}"));
                    self.send(owner, json!({"event":"ui.closed","data":{"view":id}}));
                }
            }
            Event::UiCallback(owner, data) => {
                self.send(owner, json!({"event":"ui.callback","data":data}))
            }
            Event::InjectKey(generation, key, kind, physical) => {
                if debug_keys() {
                    diag!("keys: inject to slint {key:?} {kind:?} window={generation}");
                }
                if let Some(window) = self.window_for(generation) {
                    let window = window.clone_strong();
                    if kind != Injected::Released {
                        self.input.borrow_mut().physical = physical;
                    }
                    let text = key.into();
                    window.window().dispatch_event(match kind {
                        Injected::Pressed => slint::platform::WindowEvent::KeyPressed { text },
                        Injected::Repeated => {
                            slint::platform::WindowEvent::KeyPressRepeated { text }
                        }
                        Injected::Released => slint::platform::WindowEvent::KeyReleased { text },
                    });
                    // The press was resolved (or not) by now.
                    self.input.borrow_mut().physical = None;
                }
            }
            Event::ModifiersReleased(generation, keys) => {
                if debug_keys() {
                    diag!(
                        "keys: release to slint {keys:?} window={generation} live={} shell={}",
                        self.window_for(generation).is_some(),
                        self.shell_generation
                    );
                }
                if let Some(window) = self.window_for(generation) {
                    for key in keys {
                        window
                            .window()
                            .dispatch_event(slint::platform::WindowEvent::KeyReleased {
                                text: key.into(),
                            });
                    }
                }
            }
            Event::Dropped(generation, path) => self.drops.arrived(generation, path),
            Event::OpenFiles(files, front) => self.open_files(files, front),
            Event::Settings(generation, index) => self.settings_event(generation, index),
            Event::SettingsChecked(serial, result) => self.settings_checked(serial, result),
            Event::ConfirmDone(reply, confirmed, checked) => {
                self.policy.activity(self.now_ms());
                self.dialog_busy = false;
                self.respond(reply, Ok(json!({"confirmed":confirmed,"checked":checked})));
            }
            Event::DialogDone(reply, paths) => {
                self.policy.activity(self.now_ms());
                self.dialog_busy = false;
                self.respond(
                    reply,
                    Ok(json!({"cancelled":paths.is_none(),"paths":paths.unwrap_or_default()})),
                );
            }
            Event::Exited(owner) => {
                // Its stdout closed: its process is gone, so its stop sends
                // no `runtime.stop`.
                if let Some(plugin) = self.plugins.get_mut(&owner) {
                    plugin.set_exited();
                }
                // Still running: the process ended without being stopped.
                self.record_failure(owner, "plugin process exited");
                self.process_ended(owner, crate::supervisor::ExitCause::Exited, None)
            }
            Event::Runner(runner, message) => self.runner_event(runner, &message),
            Event::RunnerExited(runner) => {
                if self.shared.as_ref().is_some_and(|s| s.link.id == runner) {
                    self.restart_shared("the shared runner exited");
                }
            }
            Event::Reaped(owner, code) => self.unit_reaped(owner, code),
            Event::Spawned(owner, result) => self.spawned(owner, result),
            Event::Recorded(owner, serial, result) => self.recorded(owner, serial, result),
            Event::OutOfMemory(owner) => {
                diag!("plugin {owner}: its JavaScript heap ran out of memory");
                self.supervisor.process_out_of_memory(Pid(owner));
            }
            Event::BackendCleaned(owner) => self.unit_cleaned(owner),
            Event::RuntimeFault(owner, error) => {
                diag!("plugin {owner}: {error}");
                let now = self.now_ms();
                let _ = self.supervisor.activation_failed(
                    Pid(owner),
                    error.chars().take(512).collect(),
                    now,
                );
                self.record_failure(owner, &error);
                self.process_ended(
                    owner,
                    crate::supervisor::ExitCause::Fault,
                    Some(error.chars().take(512).collect()),
                );
            }
        }
    }
}
