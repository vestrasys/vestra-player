//! `Host::handle`: one event of the UI thread's loop.
use super::*;

impl Host {
    pub(crate) fn handle(&mut self, event: Event) {
        match event {
            Event::Request(owner, message) => {
                if !self.plugins.contains_key(&owner) {
                    return;
                }
                if let Some(event) = message["event"].as_str() {
                    diag!("runtime {owner} {event}: {}", message["data"]);
                    let now = self.now_ms();
                    let result = match event {
                        "runtime.ready" => self.activations.runtime_ready(owner, now),
                        "runtime.failed" => self.activations.runtime_failed(
                            owner,
                            message["data"].to_string().chars().take(512).collect(),
                            now,
                        ),
                        _ => Ok(()),
                    };
                    if let Err(error) = result {
                        diag!("activation runtime event: {error}");
                    }
                    self.activation_tick();
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
                                self.activations.status(owner, now),
                                Ok(package_activation::Status::Pending { .. })
                            ) {
                                if let Err(error) = self.activations.ui_begin(owner, reply.id, now)
                                {
                                    self.respond(
                                        reply,
                                        Err(RpcError::new("activation", error.to_string())),
                                    );
                                    return;
                                }
                                self.activation_ui.insert((owner, reply.id));
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
                        self.disable(owner);
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
            Event::SidebarTab(generation, index) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                // The index is in that window's tab order.
                let tabs = match &self.floating {
                    Some(floating) if floating.generation == generation => &floating.owners,
                    _ => &self.shell_tabs,
                };
                if let Some(&owner) = tabs.get(index)
                    && self.policy.set_active(owner).is_ok()
                {
                    self.tab_clicked = true;
                    self.remember_sidebar(true);
                    self.window_tick();
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
            Event::SidebarClose(generation) => {
                if self.window_generation_live(generation) {
                    self.close_sidebar();
                    self.window_tick();
                }
            }
            Event::PluginMessage(owner, event, data) => {
                self.send(owner, json!({"event":event,"data":data}))
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
                // Still running: the process ended without being stopped.
                self.record_failure(owner, "plugin process exited");
                self.disable(owner)
            }
            Event::Runner(runner, message) => self.runner_event(runner, &message),
            Event::RunnerExited(runner) => {
                if self.shared.as_ref().is_some_and(|s| s.link.id == runner) {
                    self.restart_shared("the shared runner exited");
                }
            }
            Event::Reaped(owner) => {
                if let Some(cleanup) = self.cleanup.get_mut(&owner) {
                    cleanup.0 = true;
                }
                if let Some(directory) = self.data_removals.remove(&owner) {
                    let _ = std::fs::remove_dir_all(directory);
                }
            }
            Event::BackendCleaned(owner) => {
                if let Some(cleanup) = self.cleanup.get_mut(&owner) {
                    cleanup.1 = true;
                }
            }
            Event::RuntimeFault(owner, error) => {
                diag!("plugin {owner}: {error}");
                let now = self.now_ms();
                let _ =
                    self.activations
                        .runtime_failed(owner, error.chars().take(512).collect(), now);
                self.record_failure(owner, &error);
                self.disable(owner);
            }
        }
    }
}
