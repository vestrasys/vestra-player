//! Host commands and core actions (step seek, on-screen messages).
use super::*;

impl Host {
    pub(crate) fn execute_command(
        &mut self,
        owner: u64,
        id: &str,
    ) -> std::result::Result<(), RpcError> {
        let execution = match self.commands.resolve_execution(owner, id) {
            Ok(execution) => execution,
            Err(error) => {
                // A plugin held after a fault (supervisor.md §7): a click shows why,
                // as a key does.
                if let commands::CommandError::Disabled {
                    reason: Some(reason),
                } = &error
                    && self.commands.held(owner)
                {
                    diag!("supervisor: {id} of a held plugin: \"{reason}\"");
                    self.show_osd(&reason.clone());
                }
                return Err(RpcError::new("command", error.to_string()));
            }
        };
        if owner == bindings::HOST {
            self.host_command(id);
            return Ok(());
        }
        self.send(execution.owner,json!({"event":"commands.execute","data":{"id":execution.local_id,"revision":execution.revision}}));
        Ok(())
    }
    /// The host's own commands (`host_declarations`) and the core actions.
    pub(crate) fn register_host_commands(&mut self) -> Result<()> {
        for (id, declaration, state) in Self::host_declarations() {
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
        for (id, declaration) in core_actions::declarations(self.step_seconds()) {
            self.register_core_action(id, declaration)?;
        }
        self.core_action_states();
        Ok(())
    }
    /// The host's own commands (owner 0): toggle, enter and leave
    /// fullscreen, quit, the sidebar toggle, the command palette, with their first states
    /// (tests/sdk/host-agreement.test.ts reads this function).
    pub(crate) fn host_declarations()
    -> Vec<(&'static str, commands::Declaration, commands::CommandState)> {
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
        // IA-CORE-04 (user ruling 2026-11-01): Escape toggles full screen,
        // its only default key (up to 0.5.x Enter, Alt+Enter and F11 did,
        // and Escape was Leave Fullscreen's). An Escape a menu, dialog or
        // text field handles never reaches a binding. No mouse default (user
        // ruling 2026-10-26): a click on the video is play/pause, and a
        // double click does nothing unless the user binds it.
        fullscreen.bindings = vec!["Escape".into()];
        fullscreen.localize(|_| i18n::tr("Toggle Fullscreen"));
        // Enter and Leave Fullscreen (user ruling 2026-11-01): outside the
        // menu, no default key, each enabled only where it does something;
        // bindable in Keyboard Shortcuts.
        let mut enter = commands::Declaration::new("Enter Fullscreen", &[]);
        enter.localize(|_| i18n::tr("Enter Fullscreen"));
        let mut exit = commands::Declaration::new("Leave Fullscreen", &[]);
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
        // Host 0.7.4 (menu redesign §6): the command palette, outside the
        // menu for now; Ctrl+Shift+P (⌘⇧P on macOS too, CTRL_ACCEPTS_CMD).
        let mut palette = commands::Declaration::new("Command Palette", &[]);
        palette.localize(|_| i18n::tr("Command Palette"));
        palette.bindings = vec!["Ctrl+Shift+P".into()];
        vec![
            ("fullscreen", fullscreen, state(true, Some(false))),
            ("enter-fullscreen", enter, state(true, None)),
            ("exit-fullscreen", exit, state(false, None)),
            ("quit", quit, state(true, None)),
            // `bar_buttons::SIDEBAR` (a literal: tests/sdk/host-agreement.test.ts reads it).
            ("sidebar", sidebar, state(false, Some(false))),
            // `palette::PALETTE` (a literal: tests/sdk/host-agreement.test.ts reads it).
            ("palette", palette, state(true, None)),
        ]
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
        for (id, enabled) in fullscreen_command_states(fullscreen) {
            let _ = self.commands.update(
                bindings::HOST,
                id,
                revision,
                commands::CommandState {
                    enabled,
                    checked: None,
                    busy: false,
                    reason: None,
                },
            );
        }
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
        if id == palette::PALETTE {
            self.open_palette();
            return;
        }
        if id == "quit" {
            // Before anything waits on the GPU.
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
    /// `enter-fullscreen` enters, `exit-fullscreen` leaves. False without
    /// a window.
    pub(crate) fn window_command(&self, id: &str) -> bool {
        let Some(shell) = &self.shell else {
            return false;
        };
        let window = shell.window();
        match id {
            "fullscreen" => window.set_fullscreen(!window.is_fullscreen()),
            "enter-fullscreen" => window.set_fullscreen(true),
            "exit-fullscreen" => window.set_fullscreen(false),
            _ => {}
        }
        true
    }
}

/// Enter and Leave Fullscreen are enabled only where they do something:
/// Enter while windowed, Leave in fullscreen (`host_command_states`).
pub(crate) fn fullscreen_command_states(fullscreen: bool) -> [(&'static str, bool); 2] {
    [
        ("enter-fullscreen", !fullscreen),
        ("exit-fullscreen", fullscreen),
    ]
}

/// The one-time migration of [`migrate_fullscreen_escape`], recorded in
/// the interface state (`bindingsMigrated`).
pub(crate) const FULLSCREEN_ESCAPE_MIGRATION: &str = "fullscreen-escape-0.6.0";

/// Host 0.6.0 (coordinator decision 2026-11-01): Escape became Toggle
/// Fullscreen's default key, and a user override of `host/fullscreen` (such
/// as `[]`, saved when its defaults were Enter, Alt+Enter and F11) would
/// hide it. Once per interface state, Escape is added to that override
/// unless another command holds Escape: by an override, or by its default
/// (`default_holders`, the commands declared now that list it; at start-up
/// these are the host's own and the core actions, and no built-in plugin
/// declares Escape) without an override replacing it. The marker is
/// recorded whether or not Escape was added, so a user who removes it
/// later keeps it removed. Returns whether the state changed.
pub(crate) fn migrate_fullscreen_escape(
    overlay: &mut BTreeMap<String, Vec<String>>,
    migrated: &mut Vec<String>,
    default_holders: &[String],
) -> bool {
    const KEY: &str = "host/fullscreen";
    if migrated
        .iter()
        .any(|done| done == FULLSCREEN_ESCAPE_MIGRATION)
    {
        return false;
    }
    migrated.push(FULLSCREEN_ESCAPE_MIGRATION.into());
    let Ok(escape) = bindings::parse("Escape") else {
        return true;
    };
    let is_escape = |text: &String| bindings::parse(text).is_ok_and(|input| input == escape);
    let Some(own) = overlay.get(KEY) else {
        return true;
    };
    if own.iter().any(is_escape) {
        return true;
    }
    let by_override = overlay
        .iter()
        .any(|(key, inputs)| key != KEY && inputs.iter().any(is_escape));
    let by_default = default_holders
        .iter()
        .any(|holder| holder != KEY && !overlay.contains_key(holder));
    if !by_override && !by_default {
        overlay.entry(KEY.into()).or_default().push("Escape".into());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bindings::{HOST, Input, Lookup, Table};

    /// The table as register_host_commands fills it (the host's own
    /// commands, the core actions).
    fn host_table() -> Table {
        let mut table = Table::default();
        for (id, declaration, _) in Host::host_declarations() {
            table
                .declare(HOST, id, "host", &declaration.bindings, false, false)
                .unwrap();
        }
        for (id, declaration) in core_actions::declarations(5.) {
            table
                .declare(HOST, id, "host", &declaration.bindings, false, false)
                .unwrap();
        }
        table
    }

    fn key(name: &str) -> Input {
        bindings::parse(name).unwrap()
    }

    fn overlay(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
        entries
            .iter()
            .map(|(key, inputs)| {
                (
                    (*key).to_owned(),
                    inputs.iter().map(|input| (*input).to_owned()).collect(),
                )
            })
            .collect()
    }

    /// Enter and Leave Fullscreen start as the windowed player needs them
    /// (their registered states) and flip with the window.
    #[test]
    fn enter_and_leave_fullscreen_are_enabled_only_where_they_act() {
        let declared: Vec<(&str, bool)> = Host::host_declarations()
            .into_iter()
            .filter(|(id, ..)| id.ends_with("-fullscreen"))
            .map(|(id, _, state)| (id, state.enabled))
            .collect();
        assert_eq!(declared, fullscreen_command_states(false));
        for fullscreen in [false, true] {
            let states = fullscreen_command_states(fullscreen);
            let ids: Vec<&str> = states.iter().map(|(id, _)| *id).collect();
            assert_eq!(ids, ["enter-fullscreen", "exit-fullscreen"]);
            let enabled: Vec<bool> = states.iter().map(|(_, on)| *on).collect();
            assert_eq!(enabled, [!fullscreen, fullscreen], "{fullscreen}");
        }
    }

    /// User ruling 2026-11-01, from the real declarations: Escape is Toggle
    /// Fullscreen's only key; Enter and Leave Fullscreen have none, and
    /// Enter, Alt+Enter and F11 are free. A panel command bound to Escape
    /// still wins while its panel has the focus.
    #[test]
    fn escape_toggles_fullscreen_and_enter_and_leave_have_no_key() {
        let declarations = Host::host_declarations();
        let keys = |id: &str| {
            declarations
                .iter()
                .find(|(each, ..)| *each == id)
                .map(|(_, declaration, _)| declaration.bindings.clone())
                .unwrap()
        };
        assert_eq!(keys("fullscreen"), ["Escape"]);
        assert!(keys("enter-fullscreen").is_empty());
        assert!(keys("exit-fullscreen").is_empty());
        let mut table = host_table();
        match table.lookup(&key("Escape"), None) {
            Lookup::Fire(target) => assert_eq!(target.id, "fullscreen"),
            other => panic!("{other:?}"),
        }
        assert_eq!(table.default_holders(&key("Escape")), ["host/fullscreen"]);
        for name in ["Enter", "Alt+Enter", "F11"] {
            assert_eq!(table.lookup(&key(name), None), Lookup::Unbound, "{name}");
        }
        table
            .declare(4, "close", "p.panel", &["Escape".into()], true, false)
            .unwrap();
        match table.lookup(&key("Escape"), Some(4)) {
            Lookup::Fire(target) => assert_eq!(target.id, "close"),
            other => panic!("{other:?}"),
        }
    }

    /// Menu redesign §6: Ctrl+Shift+P opens the command palette, and no
    /// other command of the player holds it by default.
    #[test]
    fn ctrl_shift_p_opens_the_command_palette() {
        let table = host_table();
        assert_eq!(
            table.default_holders(&key("Ctrl+Shift+P")),
            ["host/palette"]
        );
        match table.lookup(&key("Ctrl+Shift+P"), None) {
            Lookup::Fire(target) => assert_eq!(target.id, palette::PALETTE),
            other => panic!("{other:?}"),
        }
        let (_, declaration, state) = Host::host_declarations()
            .into_iter()
            .find(|(id, ..)| *id == palette::PALETTE)
            .unwrap();
        assert!(declaration.path.is_empty(), "outside the menu for now");
        assert!(state.enabled);
    }

    /// M1 (coordinator decision 2026-11-01): the user's own profile has
    /// `host/fullscreen: []`; the migration gives it Escape, once.
    #[test]
    fn an_empty_fullscreen_override_gets_escape_once() {
        let holders = host_table().default_holders(&key("Escape"));
        let mut state = overlay(&[("host/fullscreen", &[])]);
        let mut migrated = Vec::new();
        assert!(migrate_fullscreen_escape(
            &mut state,
            &mut migrated,
            &holders
        ));
        assert_eq!(state, overlay(&[("host/fullscreen", &["Escape"])]));
        assert_eq!(migrated, [FULLSCREEN_ESCAPE_MIGRATION]);
        // The user removes it again: the next load keeps it removed.
        state.insert("host/fullscreen".into(), Vec::new());
        assert!(!migrate_fullscreen_escape(
            &mut state,
            &mut migrated,
            &holders
        ));
        assert_eq!(state, overlay(&[("host/fullscreen", &[])]));
        assert_eq!(migrated, [FULLSCREEN_ESCAPE_MIGRATION]);
        // No override: nothing to add, the marker is still recorded.
        let mut state = BTreeMap::new();
        let mut migrated = Vec::new();
        assert!(migrate_fullscreen_escape(
            &mut state,
            &mut migrated,
            &holders
        ));
        assert!(state.is_empty());
        assert_eq!(migrated, [FULLSCREEN_ESCAPE_MIGRATION]);
    }

    /// An override that already has Escape, or Escape held by another
    /// command (its override, or its default not overridden), is left alone.
    #[test]
    fn a_fullscreen_override_is_left_alone_when_escape_is_taken() {
        let holders = host_table().default_holders(&key("Escape"));
        let migrate = |before: BTreeMap<String, Vec<String>>, holders: &[String]| {
            let mut state = before;
            assert!(migrate_fullscreen_escape(
                &mut state,
                &mut Vec::new(),
                holders
            ));
            state
        };
        let with_escape = overlay(&[("host/fullscreen", &["F11", "Escape"])]);
        assert_eq!(migrate(with_escape.clone(), &holders), with_escape);
        let elsewhere = overlay(&[
            ("host/fullscreen", &["F11"]),
            ("vestra.playlist/close", &["Escape"]),
        ]);
        assert_eq!(migrate(elsewhere.clone(), &holders), elsewhere);
        let others = ["host/fullscreen".to_owned(), "p.panel/close".to_owned()];
        let by_default = overlay(&[("host/fullscreen", &[])]);
        assert_eq!(migrate(by_default.clone(), &others), by_default);
        // That command's override drops Escape: it is free again.
        let freed = overlay(&[("host/fullscreen", &[]), ("p.panel/close", &["X"])]);
        assert_eq!(migrate(freed, &others)["host/fullscreen"], ["Escape"]);
        // Shift+Escape elsewhere is another key.
        let shifted = overlay(&[("host/fullscreen", &[]), ("x/y", &["Shift+Escape"])]);
        assert_eq!(migrate(shifted, &holders)["host/fullscreen"], ["Escape"]);
    }
}
