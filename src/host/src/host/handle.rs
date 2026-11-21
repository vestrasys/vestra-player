//! `Host::handle`: one event of the loop, routed (plan step S2f(1)). The
//! 26 UI-originated events go to the interface's part (`ui_event`), which
//! reads and writes the interface's state and reports what the core has to
//! do (`UiReport::Event`, `UiReport::Input`), delivered in place at the
//! point where the core's work ran before; every other event is the
//! core's (`Core::handle`).
use super::messages::{CommandFrom, PluginLine, UiEvent, UiInput, UiReport};
use super::*;

impl Host {
    /// An event of the core's queue. Plan step S3a(3): the interface's
    /// events have a queue of their own ([`Host::ui_events_tick`]); one
    /// that still reaches the core's queue is the interface's all the same
    /// and runs here, on this thread (none is known: every UI callback
    /// sends through `UiEvents`).
    pub(crate) fn handle(&mut self, event: Event) {
        match UiEvent::new(event) {
            Ok(event) => self.ui_event(event.into_event()),
            // The core's, through the driver (plan step S3a(1),
            // `driver.rs`): the core's handling, then what it issued.
            Err(event) => self.deliver_event(event),
        }
    }
    /// The interface's own events (plan step S3a(3), the S3a brief's item
    /// 1), at its wake after the core→UI queue was applied, so a key is
    /// judged by the tables pushed before it (R8). A line from the
    /// interface's own threads that is the core's (`PluginMessage`,
    /// `Broadcast` from a view's or a settings page's callbacks) goes on to
    /// the core's queue, in its order.
    pub(crate) fn ui_events_tick(&mut self) {
        let Ok(first) = self.ui_events.try_recv() else {
            return;
        };
        // Plan step S3a(4): the core→UI queue first (R8: a key behind a
        // table snapshot is judged by it), also between the core's wakes.
        self.apply_queue();
        let mut next = Some(first);
        while let Some(event) = next.take() {
            match UiEvent::new(event) {
                Ok(event) => self.ui_event(event.into_event()),
                Err(event) => {
                    let _ = self.to_core.send(event);
                }
            }
            next = self.ui_events.try_recv().ok();
        }
    }
}
impl Core {
    /// An event of the core: a plugin, the playback worker, the reaper, the
    /// spawner, the recorder, the settings check, a dialog's thread. What it
    /// asks of the interface waits in the outbox (plan step S2f(3)).
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
                    // The interface's copy follows at once (plan step
                    // S2c(7)), before anything below reads it.
                    self.source_shown();
                    // What a restart after a lost GPU device reopens.
                    *LAST_SOURCE
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) = data.clone();
                    if previous["phase"] != data["phase"] {
                        diag!("source: phase {} at +{} ms", data["phase"], self.now_ms());
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
                    // What it shows is the core's to decide (plan step
                    // S2d(4)): the interface shows it.
                    let shown = self.source_display_next();
                    self.ui_command(messages::UiCommand::Snapshot(messages::UiSnapshot::Source(
                        shown,
                    )));
                    self.core_action_states();
                    // The discrete source events this state adds, after it:
                    // a listener of both reads the state first.
                    let derived = crate::source_events::derive(&previous, &data);
                    self.broadcast(&event, data);
                    for (name, data) in derived {
                        diag!(
                            "source: {name} {} at +{} ms",
                            data["state"]["stamp"]["generation"],
                            self.now_ms()
                        );
                        self.broadcast(name, data);
                    }
                    return;
                }
                if event == "resolvers.fileTypes" {
                    self.file_types = data["fileTypes"].clone();
                }
                self.broadcast(&event, data)
            }
            Event::OwnerResources(owner, answer) => self.resources_answer(owner, &answer),
            Event::Position(report) => self.triggers_position(report),
            Event::SubtitleTrack(track) => self.triggers_subtitle(track),
            // Plan step S3a(3): a report of the interface's, from the queue.
            Event::Report(report) => self.report(*report),
            // Plan step S3a(4): the interface's quit, acted on at the end of
            // the core's turn (`driver::run_core`).
            Event::CoreQuit(quit) => self.quit_asked = Some(quit),
            Event::PluginMessage(owner, event, data) => {
                self.send(owner, json!({"event":event,"data":data}))
            }
            Event::OpenFiles(files, front) => self.open_files(files, front),
            Event::SettingsChecked(serial, result) => self.settings_checked(serial, result),
            Event::ConfirmDone(reply, confirmed, checked) => {
                self.ui_command(messages::UiCommand::Activity);
                self.dialog_busy = false;
                self.confirm_open = false;
                self.respond(reply, Ok(json!({"confirmed":confirmed,"checked":checked})));
            }
            Event::SourceLocate(reply, fields) => {
                if let Err(error) = crate::capabilities::locate_dialog(self, &reply, fields) {
                    self.respond(reply, Err(error));
                }
            }
            Event::SourceLocated(reply, mut fields, path) => {
                self.ui_command(messages::UiCommand::Activity);
                self.dialog_busy = false;
                match path.as_deref().map(|path| path.to_str()) {
                    None => self.respond(reply, Ok(Value::Null)),
                    Some(None) => self.respond(
                        reply,
                        Err(RpcError::new(
                            "unsupported",
                            "the file's path is not Unicode",
                        )),
                    ),
                    Some(Some(path)) => {
                        fields["path"] = json!(path);
                        let sent = self
                            .backend
                            .commands
                            .send(crate::playback::backend::Command {
                                reply: reply.clone(),
                                method: "owner.sourceLocated".into(),
                                params: fields,
                            });
                        if sent.is_err() {
                            self.respond(
                                reply,
                                Err(RpcError::new("backend", "playback worker stopped")),
                            );
                        }
                    }
                }
            }
            Event::DialogDone(reply, paths) => {
                self.ui_command(messages::UiCommand::Activity);
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
            ui => unreachable!("{:?} is the interface's (`ui_event`)", ui.label()),
        }
    }
}
impl Host {
    /// The interface's part of one of the 26 UI-originated events (plan
    /// step S2f(1)). Where the core has work, it is reported at the point
    /// where it ran before: the event itself (`UiReport::Event`) or what
    /// the interface derived from it (`UiReport::Input`).
    fn ui_event(&mut self, event: Event) {
        match event {
            #[cfg(feature = "producer-audio")]
            Event::PhysicalInput(generation, params) => {
                if generation == self.shell_generation {
                    self.report_event(Event::PhysicalInput(generation, params));
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
                        self.ui_report(UiReport::Input(UiInput::MenuReset));
                        self.open_menu(generation, MenuMode::Main);
                    }
                    MenuAction::Back => {
                        let mut path = self.menu_path.clone();
                        path.pop();
                        self.set_menu_path(path);
                    }
                    // A press outside the approval sheet answers Don't Run.
                    MenuAction::Dismiss if self.menu_mode == MenuMode::Review => {
                        self.review_action("decline");
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
                            // The window it came from, as the interface knows
                            // its windows; the core records it (`origins`).
                            let origin = self.origin_of(self.menu_generation);
                            self.close_menu();
                            self.ui_report(UiReport::Input(UiInput::Command {
                                from: CommandFrom::Menu,
                                owner,
                                id,
                                origin,
                            }));
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
            Event::Binding(generation, owner, id) => {
                if !self.window_generation_live(generation) {
                    return;
                }
                input_trace::mark(&format!("host loop: binding → {owner}/{id}"));
                self.policy.activity(self.now_ms());
                let origin = self.origin_of(generation);
                self.ui_report(UiReport::Input(UiInput::Command {
                    from: CommandFrom::Key,
                    owner,
                    id,
                    origin,
                }));
            }
            Event::SidebarResized(generation, width) => {
                if generation != self.shell_generation {
                    return;
                }
                self.policy.set_sidebar_width(width);
                self.remembered.sidebar = Some(ui_state::Sidebar { width });
                self.remember(self.now_ms());
            }
            Event::BarButton(generation, token) => self.bar_click(generation, &token),
            Event::BarMore(generation, x, y) => self.bar_more(generation, x, y),
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
                self.remembered.adjust_slot = Some(slot);
                self.remember(self.now_ms());
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
            Event::MainClosed => self.report_event(Event::MainClosed),
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
            Event::LeadStrip(generation, action) => {
                if generation == self.shell_generation {
                    self.lead_strip_action(&action);
                }
            }
            Event::ReviewSheet(generation, action) => {
                if generation == self.shell_generation {
                    self.policy.activity(self.now_ms());
                    self.review_action(&action);
                }
            }
            Event::ViewClosed(owner, id) => {
                if let Some(view) = self.views.remove(&(owner, id.clone())) {
                    let _ = view.instance.hide();
                    self.images.release_view(owner, &format!("w:{id}"));
                    // The core tells the plugin (`ui.closed`).
                    self.report_event(Event::ViewClosed(owner, id));
                }
            }
            Event::UiCallback(owner, data) => {
                // A confirmation it asks for now belongs to this window.
                if let Some(view) = data["view"].as_str() {
                    self.acted_in_view(owner, view);
                }
                // The core writes the plugin's line (`ui.callback`, R6).
                self.report_event(Event::UiCallback(owner, data))
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
            Event::Settings(generation, index) => self.settings_event(generation, index),
            Event::CommandRunAnswered(reply, yes, remember) => {
                self.policy.activity(self.now_ms());
                self.report_event(Event::CommandRunAnswered(reply, yes, remember));
            }
            core => unreachable!("{:?} is the core's (`Core::handle`)", core.label()),
        }
    }
    /// Report a UI event whose core part needs only the event
    /// (`UiReport::Event`).
    fn report_event(&mut self, event: Event) {
        if let Ok(event) = UiEvent::new(event) {
            self.ui_report(UiReport::Event(event));
        }
    }
    /// The window of `generation` as the origin of what the user did there
    /// (`origins::of_generation`; `None`: no live window of it).
    pub(crate) fn origin_of(&self, generation: u64) -> Option<origins::Origin> {
        let floating = self.floating.as_ref().map(|floating| floating.generation);
        origins::of_generation(generation, self.shell_generation, floating)
    }
}
impl Core {
    /// The core's part of a UI event the interface reported whole
    /// (`UiReport::Event`, plan step S2f(1)).
    pub(crate) fn core_ui_event(&mut self, event: UiEvent) {
        match event.into_event() {
            #[cfg(feature = "producer-audio")]
            Event::PhysicalInput(_, params) => self.physical_input(params),
            Event::MainClosed => self.host_command("quit"),
            Event::ViewClosed(owner, view) => {
                self.send(owner, json!({"event":"ui.closed","data":{"view":view}}));
            }
            Event::UiCallback(owner, callback) => {
                self.send(owner, json!({"event":"ui.callback","data":callback}))
            }
            Event::CommandRunAnswered(reply, yes, remember) => {
                crate::capabilities::command_run_answered(self, reply, yes, remember);
            }
            other => unreachable!("{:?} has no core part", other.label()),
        }
    }
    /// The core's part of what the interface derived from a UI event
    /// (`UiReport::Input`, plan step S2f(1)).
    pub(crate) fn core_input(&mut self, input: UiInput) {
        match input {
            UiInput::Command {
                from: CommandFrom::Menu,
                owner,
                id,
                origin,
            } => {
                if let Some(origin) = origin {
                    self.act(owner, origin);
                }
                if let Err(error) = self.execute_command(owner, &id) {
                    diag!("menu execute: {:?}", error);
                }
            }
            UiInput::Command {
                from: CommandFrom::Key,
                owner,
                id,
                origin,
            } => match self.commands.resolve_execution(owner, &id) {
                // A key never fails silently: the command's reason shows.
                Err(commands::CommandError::Disabled {
                    reason: Some(reason),
                }) => self.show_osd(&reason),
                _ => {
                    if let Some(origin) = origin {
                        self.act(owner, origin);
                    }
                    if let Err(error) = self.execute_command(owner, &id) {
                        diag!("binding execute: {error:?}");
                    }
                }
            },
            UiInput::Command {
                from: CommandFrom::Bar,
                owner,
                id,
                origin,
            } => self.bar_command(owner, &id, origin),
            UiInput::Command {
                from: CommandFrom::Osd,
                owner,
                id,
                origin,
            } => self.osd_command_run(owner, &id, origin),
            UiInput::Command {
                from: CommandFrom::Palette,
                owner,
                id,
                origin,
            } => self.palette_command(owner, &id, origin),
            UiInput::HostCommand(id) => self.host_command(id),
            UiInput::Lead {
                query,
                keep,
                status,
            } => self.lead_event(query, keep, status),
            UiInput::MenuReset => {
                #[cfg(feature = "producer-audio")]
                self.physical_revoke(producer_input::RevokeReason::Menu);
            }
            UiInput::PageShown(owner) => self.page_shown(owner),
            UiInput::TabShown(owner) => self.panel_shown(owner),
            UiInput::TabWork(owner) => self.tab_work(owner),
            UiInput::Line(PluginLine::UiError { owner, data }) => {
                self.send(owner, json!({"event":"ui.error","data":data}));
            }
            UiInput::Line(PluginLine::UiCallback { owner, data }) => {
                self.send(owner, json!({"event":"ui.callback","data":data}));
            }
            UiInput::Line(PluginLine::HostError(data)) => self.broadcast("host.error", data),
            UiInput::WindowTick { fullscreen } => {
                self.host_command_states(fullscreen);
                self.backdrop_cover_art();
            }
            UiInput::ColumnCommands(states) => self.column_command_states(states),
            UiInput::MountTick => self.release_held_mounts(),
            UiInput::PanelBack(key) => self.panel_back(key),
            // Plan step S2f(3): the interface shows a `dialogs.confirm` as a
            // system dialog; busy before it shows.
            UiInput::SystemConfirm => {
                self.dialog_busy = true;
                self.confirm_open = true;
                #[cfg(feature = "producer-audio")]
                self.physical_revoke(producer_input::RevokeReason::Dialog);
            }
            // The batch failed (S3a(3) review B1/B2: its outcome ends its
            // wait; the core tries the changes one at a time).
            UiInput::MountRetry(changes) => self.batch_retry(changes),
            // Plan step S3a(1): what the interface wrote in the core's data,
            // or called on the core, in place before.
            UiInput::Mounts(record) => self.mounts = record,
            UiInput::PanelPlugins => self.note_panel_plugins(),
            UiInput::Mounted(owner) => self.clear_warning(owner, "slot-occupied"),
            UiInput::Stored {
                owner,
                id,
                property,
                write,
            } => self.settings_stored(owner, &id, property, write),
            UiInput::PipAction { action, step } => {
                if let Err(error) = self.run_core_action_with(action, None, step) {
                    diag!("pip: {action}: {}", error.message);
                }
            }
            #[cfg(feature = "producer-audio")]
            UiInput::Revoke(reason) => self.physical_revoke(reason),
        }
    }
}
impl Host {
    /// A plugin line the interface's work produced (design §5.3, group E):
    /// reported at the point where the interface wrote it before, so the
    /// core writes it at the same place on the wire (R6, plan step
    /// S2f(1)).
    pub(crate) fn report_line(&mut self, line: PluginLine) {
        self.ui_report(UiReport::Input(UiInput::Line(line)));
    }
    /// `ui.error` of `error` for `owner`, reported (`report_line`).
    pub(crate) fn report_ui_error(&mut self, owner: u64, error: &RpcError) {
        self.report_line(PluginLine::UiError {
            owner,
            data: rpc::ui_error_data(error),
        });
    }
}
impl Core {
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
}
