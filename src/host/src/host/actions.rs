//! Host commands and core actions (step seek, on-screen messages).
use super::*;

impl Host {
    pub(crate) fn execute_command(
        &mut self,
        owner: u64,
        id: &str,
    ) -> std::result::Result<(), RpcError> {
        let execution = self
            .commands
            .resolve_execution(owner, id)
            .map_err(|e| RpcError::new("command", e.to_string()))?;
        if owner == bindings::HOST {
            self.host_command(id);
            return Ok(());
        }
        self.send(execution.owner,json!({"event":"commands.execute","data":{"id":execution.local_id,"revision":execution.revision}}));
        Ok(())
    }
    /// The host's own commands (owner 0): fullscreen, exit fullscreen, quit.
    pub(crate) fn register_host_commands(&mut self) -> Result<()> {
        let state = |enabled: bool, checked: Option<bool>| commands::CommandState {
            enabled,
            checked,
            busy: false,
            reason: None,
        };
        let mut fullscreen =
            commands::Declaration::new("Toggle Fullscreen", &["Toggle Fullscreen"]);
        fullscreen.group = "screen".into();
        fullscreen.order = 700;
        // IA-CORE-04: Enter and Alt+Enter toggle full screen; F11 too. No
        // mouse default (user ruling 2026-10-26): a click on the video is
        // play/pause, and a double click does nothing unless the user binds it.
        fullscreen.bindings = vec!["Enter".into(), "Alt+Enter".into(), "F11".into()];
        fullscreen.localize(|_| i18n::tr("Toggle Fullscreen"));
        let mut exit = commands::Declaration::new("Leave Fullscreen", &[]);
        exit.bindings = vec!["Escape".into()];
        exit.localize(|_| i18n::tr("Leave Fullscreen"));
        let mut quit = commands::Declaration::new("Quit", &["Quit"]);
        quit.localize(|_| i18n::tr("Quit"));
        quit.group = "exit".into();
        quit.order = 990;
        // Windows and Linux menus show no `Cmd` binding (`label_shown`).
        quit.bindings = vec!["Cmd+Q".into()];
        // Player 0.5.10: the bottom bar's sidebar toggle (the rightmost cell
        // of `BarButtons`, bar_buttons.rs); outside the menu, no default key
        // (F6 is the playlist's), bindable in Keyboard Shortcuts.
        let mut sidebar = commands::Declaration::new("Toggle Sidebar", &[]);
        sidebar.localize(|_| i18n::tr("Toggle Sidebar"));
        for (id, declaration, state) in [
            ("fullscreen", fullscreen, state(true, Some(false))),
            ("exit-fullscreen", exit, state(false, None)),
            ("quit", quit, state(true, None)),
            // `bar_buttons::SIDEBAR` (a literal: tests/sdk/host-agreement.test.ts reads it).
            ("sidebar", sidebar, state(false, Some(false))),
        ] {
            self.input
                .borrow_mut()
                .table
                .declare(
                    bindings::HOST,
                    id,
                    "host",
                    &declaration.bindings,
                    false,
                    false,
                )
                .map_err(anyhow::Error::msg)?;
            self.commands
                .register(bindings::HOST, "host", id.into(), 1, declaration, state)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        self.host_revision = 1;
        {
            for (id, declaration) in core_actions::declarations(self.step_seconds()) {
                self.register_core_action(id, declaration)?;
            }
            self.core_action_states();
        }
        Ok(())
    }
    pub(crate) fn register_core_action(
        &mut self,
        id: &str,
        declaration: commands::Declaration,
    ) -> Result<()> {
        self.input
            .borrow_mut()
            .table
            .declare(
                bindings::HOST,
                id,
                "host",
                &declaration.bindings,
                false,
                declaration.repeat,
            )
            .map_err(anyhow::Error::msg)?;
        let state = core_actions::state(id, &core_actions::Facts::default(), false);
        self.commands
            .register(
                bindings::HOST,
                "host",
                id.into(),
                self.host_revision.max(1),
                declaration,
                state,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    /// The first-tier relative seek: the newest `setStepSeek`, else 5 seconds.
    pub(crate) fn step_seconds(&self) -> f64 {
        self.step_seek
            .values()
            .max_by_key(|(sequence, ..)| *sequence)
            .map_or(core_actions::DEFAULT_STEP, |(_, seconds, _)| *seconds)
    }
    /// Whether step seeks land on keyframes: the newest `setStepSeek`'s
    /// `keyframes` (written as a pair with the seconds), else exact.
    pub(crate) fn step_keyframes(&self) -> bool {
        self.step_seek
            .values()
            .max_by_key(|(sequence, ..)| *sequence)
            .is_some_and(|(.., keyframes)| *keyframes)
    }
    /// The seek menu titles carry the step: re-register them when it changes.
    pub(crate) fn step_seek_changed(&mut self, before: f64) {
        let after = self.step_seconds();
        if after == before {
            return;
        }
        for (id, declaration) in core_actions::declarations(after) {
            if id == "seek-backward" || id == "seek-forward" {
                let _ = self.commands.remove(bindings::HOST, id);
                if let Err(error) = self.register_core_action(id, declaration) {
                    diag!("host.error core action {id}: {error}");
                }
            }
        }
        self.core_action_states();
    }
    /// The last source state with the sequence flags merged in.
    pub(crate) fn core_facts(&self) -> core_actions::Facts {
        let mut state = self.last_source.clone();
        self.patch_source(&mut state);
        core_actions::Facts::from_state(&state)
    }
    /// Publish changed enabled/checked/busy states of the registered actions.
    pub(crate) fn core_action_states(&mut self) {
        let facts = self.core_facts();
        for id in core_actions::ACTIONS {
            if core_actions::WINDOW_ACTIONS.contains(&id) {
                continue;
            }
            let state = core_actions::state(id, &facts, self.actions_busy.contains(id));
            if self
                .commands
                .get(bindings::HOST, id)
                .is_some_and(|entry| entry.state != state)
            {
                self.host_revision += 1;
                let _ = self
                    .commands
                    .update(bindings::HOST, id, self.host_revision, state);
            }
        }
    }
    /// Run a core action for a binding, the menu or `host.run`. `reply` (a
    /// plugin's `host.run`) is answered when it finished. `Ok(true)` means
    /// the action is done and the caller answers now (a sequence step
    /// handed to its provider, a repeat dropped while the same action runs,
    /// a window action); `Ok(false)` means the backend answers `reply`
    /// itself later (`host.run` then returns no reply). A disabled action
    /// is `unavailable`.
    pub(crate) fn run_core_action(
        &mut self,
        action: &str,
        reply: Option<&Reply>,
    ) -> std::result::Result<bool, RpcError> {
        self.run_core_action_with(action, reply, None)
    }
    /// `run_core_action` with a seek step other than the user's (the PiP
    /// window's skip interval).
    pub(crate) fn run_core_action_with(
        &mut self,
        action: &str,
        reply: Option<&Reply>,
        step: Option<f64>,
    ) -> std::result::Result<bool, RpcError> {
        if core_actions::WINDOW_ACTIONS.contains(&action) {
            // `host.run("fullscreen")` (host 0.5.2): what the Toggle
            // Fullscreen key does, answered at once.
            return if self.window_command(action) {
                Ok(true)
            } else {
                Err(RpcError::new("unavailable", "the player has no window"))
            };
        }
        let facts = self.core_facts();
        let state = core_actions::state(action, &facts, false);
        if !state.enabled {
            return Err(RpcError::new(
                "unavailable",
                state.reason.unwrap_or_else(|| "unavailable".into()),
            ));
        }
        if matches!(action, "previous" | "next") {
            let (provider, ..) = self
                .sequence
                .ok_or_else(|| RpcError::new("unavailable", "no sequence provider"))?;
            self.send(
                provider,
                json!({"event":"sequence.step","data":{"direction":action}}),
            );
            input_trace::mark(&format!(
                "host loop: sequence.step {action} sent to plugin {provider}"
            ));
            return Ok(true);
        }
        if core_actions::exclusive(action) && !self.actions_busy.insert(action.to_owned()) {
            // The same action is still running: this trigger is dropped.
            return Ok(true);
        }
        self.core_action_states();
        let reply = reply.cloned().unwrap_or(Reply {
            owner: bindings::HOST,
            id: 0,
        });
        self.backend
            .commands
            .send(Command {
                reply,
                method: "host.action".into(),
                params: json!({"action":action,"step":step.unwrap_or_else(|| self.step_seconds()),
                    "keyframes":self.step_keyframes(),
                    "sequence":{"next":facts.next,"previous":facts.previous}}),
            })
            .map_err(|_| RpcError::new("backend", "playback worker stopped"))?;
        input_trace::mark(&format!("host loop: {action} sent to the playback worker"));
        Ok(false)
    }
    pub(crate) fn action_done(
        &mut self,
        reply: Reply,
        action: &str,
        result: std::result::Result<Value, RpcError>,
    ) {
        self.actions_busy.remove(action);
        match &result {
            Ok(value) => {
                if let Some(text) = value["osd"].as_str() {
                    self.show_osd(text);
                }
            }
            // Failures show on the video, like the playback bar did.
            Err(error) if reply.owner == bindings::HOST => self.show_osd(&error.message),
            Err(_) => {}
        }
        self.core_action_states();
        if reply.owner != bindings::HOST {
            self.respond(reply, result.map(|_| Value::Null));
        }
    }
    /// One OSD line; nothing without a window.
    pub(crate) fn show_osd(&mut self, text: &str) {
        let text: String = text.replace(['\n', '\r'], " ").chars().take(200).collect();
        if let Some(shell) = &self.shell
            && !text.trim().is_empty()
        {
            let _ = shell.set_property("osd-text", SValue::String(text.as_str().into()));
            let _ = shell.set_property("osd-visible", true.into());
            self.osd_until = Some(self.now_ms() + OSD_MS);
        }
    }
    pub(crate) fn host_command_states(&mut self, fullscreen: bool) {
        let checked = self
            .commands
            .get(bindings::HOST, "fullscreen")
            .and_then(|entry| entry.state.checked);
        if checked == Some(fullscreen) {
            return;
        }
        self.host_revision += 1;
        let revision = self.host_revision;
        let _ = self.commands.update(
            bindings::HOST,
            "fullscreen",
            revision,
            commands::CommandState {
                enabled: true,
                checked: Some(fullscreen),
                busy: false,
                reason: None,
            },
        );
        let _ = self.commands.update(
            bindings::HOST,
            "exit-fullscreen",
            revision,
            commands::CommandState {
                enabled: fullscreen,
                checked: None,
                busy: false,
                reason: None,
            },
        );
    }
    pub(crate) fn host_command(&mut self, id: &str) {
        if core_actions::ACTIONS.contains(&id) && !core_actions::WINDOW_ACTIONS.contains(&id) {
            if let Err(error) = self.run_core_action(id, None) {
                diag!("core action {id}: {}", error.message);
            }
            return;
        }
        if id == bar_buttons::SIDEBAR {
            self.toggle_sidebar();
            return;
        }
        if id == "quit" {
            // Before anything waits on the GPU (record §11.6).
            platform::release_gpu_waits();
            crate::stop_handoffs();
            self.ui_state.flush();
            let _ = slint::quit_event_loop();
            return;
        }
        self.window_command(id);
    }
    /// The main window's commands, for a key, the menu and `host.run`:
    /// `fullscreen` toggles (in when windowed, out when fullscreen),
    /// `exit-fullscreen` leaves. False without a window.
    pub(crate) fn window_command(&self, id: &str) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        let window = shell.window();
        match id {
            "fullscreen" => window.set_fullscreen(!window.is_fullscreen()),
            "exit-fullscreen" => window.set_fullscreen(false),
            _ => {}
        }
        true
    }
}
