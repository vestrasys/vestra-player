//! Host commands and core actions (step seek, on-screen messages).
use super::*;

/// Command Palette: Ask Again Before Plugin Commands (Ask Vestra UX step 4).
pub(crate) const ASK_AGAIN: &str = "ask-again";

impl Host {
    pub(crate) fn execute_command(
        &mut self,
        owner: u64,
        id: &str,
    ) -> std::result::Result<(), RpcError> {
        let execution = match self.core.commands.resolve_execution(owner, id) {
            Ok(execution) => execution,
            Err(error) => {
                // A plugin held after a fault (supervisor.md §7): a click shows why,
                // as a key does.
                if let commands::CommandError::Disabled {
                    reason: Some(reason),
                } = &error
                    && self.core.commands.held(owner)
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
            self.core
                .bindings_mut()
                .declare(
                    bindings::HOST,
                    id,
                    "host",
                    &declaration.bindings,
                    false,
                    false,
                )
                .map_err(anyhow::Error::msg)?;
            self.core
                .commands
                .register(bindings::HOST, "host", id.into(), 1, declaration, state)
                .map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        self.core.host_revision = 1;
        for (id, declaration) in core_actions::declarations(self.core.step_seconds()) {
            self.register_core_action(id, declaration)?;
        }
        self.core.core_action_states();
        Ok(())
    }
    /// The host's own commands (owner 0): toggle, enter and leave
    /// fullscreen, quit, the sidebar toggle, the command palette, each with
    /// its category (`core_actions::CATEGORIES`), with their first states
    /// (tests/sdk/host-agreement.test.ts reads this function).
    pub(crate) fn host_declarations()
    -> Vec<(&'static str, commands::Declaration, commands::CommandState)> {
        let state = |enabled: bool, checked: Option<bool>| commands::CommandState {
            enabled,
            checked,
            busy: false,
            reason: None,
        };
        // The short menu (user ruling 2026-11-11): out of the right-click
        // menu; Esc, the bottom bar's fullscreen button and the palette keep
        // it.
        let mut fullscreen = commands::Declaration::new("Toggle Fullscreen", &[]);
        // IA-CORE-04 (user ruling 2026-11-01): Escape toggles full screen,
        // its only default key (up to 0.5.x Enter, Alt+Enter and F11 did,
        // and Escape was Leave Fullscreen's). An Escape a menu, dialog or
        // text field handles never reaches a binding. No mouse default (user
        // ruling 2026-10-26): a click on the video is play/pause, and a
        // double click does nothing unless the user binds it.
        fullscreen.bindings = vec!["Escape".into()];
        fullscreen.localize(|_| i18n::tr("Toggle Fullscreen"));
        core_actions::categorize(&mut fullscreen, "Window");
        // Enter and Leave Fullscreen (user ruling 2026-11-01): outside the
        // menu, no default key, each enabled only where it does something;
        // bindable in Keyboard Shortcuts.
        let mut enter = commands::Declaration::new("Enter Fullscreen", &[]);
        enter.localize(|_| i18n::tr("Enter Fullscreen"));
        core_actions::categorize(&mut enter, "Window");
        let mut exit = commands::Declaration::new("Leave Fullscreen", &[]);
        exit.localize(|_| i18n::tr("Leave Fullscreen"));
        core_actions::categorize(&mut exit, "Window");
        let mut quit = commands::Declaration::new("Quit", &["Quit"]);
        quit.localize(|_| i18n::tr("Quit"));
        // With Open File and Reopen, as File > Exit in most desktop programs.
        core_actions::categorize(&mut quit, "File");
        // The short menu (user ruling 2026-11-11): last, in the settings
        // group with Tools ▸ and Options… (830), no separator above it.
        quit.group = "settings-main".into();
        quit.order = 990;
        // Windows and Linux menus show no `Cmd` binding (`label_shown`).
        quit.bindings = vec!["Cmd+Q".into()];
        // Player 0.5.10: the bottom bar's sidebar toggle (the rightmost cell
        // of the player's strip, bar_buttons.rs); no default key (F6 is the
        // playlist's), bindable in Keyboard Shortcuts. It reopens the sidebar
        // at its last tab. One title for the strip and the palette, as
        // Command Palette…. Out of the right-click menu since the short menu
        // (user ruling 2026-11-11; a top-level item from 2026-11-08).
        let mut sidebar = commands::Declaration::new("Sidebar", &[]);
        sidebar.localize(|_| i18n::tr("Sidebar"));
        core_actions::categorize(&mut sidebar, "Window");
        // Host 0.7.4 (menu redesign §6): the command palette; Ctrl+Shift+P
        // (⌘⇧P on macOS too, CTRL_ACCEPTS_CMD), and the magnifier of the
        // bottom-bar strip (step 3a, bar_buttons.rs). The short menu (user
        // ruling 2026-11-11): Tools ▸ (`commands::MENU_ANCHORS`), last of the
        // windows group after Vestra Player AI… (820) and Plugin Store…
        // (822), above About Vestra Player… (880); the ellipsis because it
        // opens a view.
        let mut palette =
            commands::Declaration::new("Command Palette…", &["Tools", "Command Palette…"]);
        palette.localize(|_| i18n::tr("Command Palette…"));
        // The way to find every other command, next to About and Check for Updates.
        core_actions::categorize(&mut palette, "Help");
        palette.group = "windows".into();
        palette.order = 829;
        palette.bindings = vec!["Ctrl+Shift+P".into()];
        // Host 0.7.6 (menu redesign step 3b): the adjust column's toggle, the
        // sliders cell of the bottom-bar strip between the palette and the
        // sidebar toggle; no default key (the panels' own commands have
        // Alt+1…4), bindable in Keyboard Shortcuts; it reopens the column at
        // its last tab. Out of the right-click menu since the short menu
        // (user ruling 2026-11-11; a top-level item from 2026-11-08).
        let mut adjust = commands::Declaration::new("Adjustments and Effects", &[]);
        adjust.localize(|_| i18n::tr("Adjustments and Effects"));
        core_actions::categorize(&mut adjust, "Window");
        // Ask Vestra UX step 4 (H4): forget every "Don't ask again" of the
        // approval sheet, so `commands.run` callers ask again before any
        // command of a plugin that is not built in. In the palette only (no
        // menu path), no default key.
        let mut ask_again = commands::Declaration::new("Ask Again Before Plugin Commands", &[]);
        ask_again.localize(|_| i18n::tr("Ask Again Before Plugin Commands"));
        core_actions::categorize(&mut ask_again, "Command Palette");
        vec![
            ("fullscreen", fullscreen, state(true, Some(false))),
            ("enter-fullscreen", enter, state(true, None)),
            ("exit-fullscreen", exit, state(false, None)),
            ("quit", quit, state(true, None)),
            // `bar_buttons::SIDEBAR` (a literal: tests/sdk/host-agreement.test.ts reads it).
            ("sidebar", sidebar, state(false, Some(false))),
            // `palette::PALETTE` (a literal: tests/sdk/host-agreement.test.ts reads it).
            ("palette", palette, state(true, None)),
            // `bar_buttons::ADJUST` (a literal: tests/sdk/host-agreement.test.ts reads it).
            ("adjust", adjust, state(false, Some(false))),
            // `ASK_AGAIN`.
            ("ask-again", ask_again, state(true, None)),
        ]
    }
    pub(crate) fn register_core_action(
        &mut self,
        id: &str,
        declaration: commands::Declaration,
    ) -> Result<()> {
        self.core
            .bindings_mut()
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
        self.core
            .commands
            .register(
                bindings::HOST,
                "host",
                id.into(),
                self.core.host_revision.max(1),
                declaration,
                state,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }
    /// The seek menu titles carry the step: re-register them when it changes.
    pub(crate) fn step_seek_changed(&mut self, before: f64) {
        let after = self.core.step_seconds();
        if after == before {
            return;
        }
        for (id, declaration) in core_actions::declarations(after) {
            if id == "seek-backward" || id == "seek-forward" {
                let _ = self.core.commands.remove(bindings::HOST, id);
                if let Err(error) = self.register_core_action(id, declaration) {
                    diag!("host.error core action {id}: {error}");
                }
            }
        }
        self.core.core_action_states();
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
        let facts = self.core.core_facts();
        let state = core_actions::state(action, &facts, false);
        if !state.enabled {
            return Err(RpcError::new(
                "unavailable",
                state.reason.unwrap_or_else(|| "unavailable".into()),
            ));
        }
        if matches!(action, "previous" | "next") {
            let (provider, ..) = self
                .core
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
        if core_actions::exclusive(action) && !self.core.actions_busy.insert(action.to_owned()) {
            // The same action is still running: this trigger is dropped.
            return Ok(true);
        }
        self.core.core_action_states();
        let reply = reply.cloned().unwrap_or(Reply {
            owner: bindings::HOST,
            id: 0,
        });
        self.core
            .backend
            .commands
            .send(Command {
                reply,
                method: "host.action".into(),
                // `queuedUs`: a slow way to the worker is logged (`stall.rs`).
                params: json!({"action":action,"queuedUs":super::stall::clock_us(),
                    "step":step.unwrap_or_else(|| self.core.step_seconds()),
                    "keyframes":self.core.step_keyframes(),
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
        self.core.actions_busy.remove(action);
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
        self.core.core_action_states();
        if reply.owner != bindings::HOST {
            self.respond(reply, result.map(|_| Value::Null));
        }
    }
    /// One OSD line; nothing without a window.
    pub(crate) fn show_osd(&mut self, text: &str) {
        let text: String = text.replace(['\n', '\r'], " ").chars().take(200).collect();
        if !text.trim().is_empty() {
            self.put_osd(&text, None, None);
        }
    }
    /// Show one OSD line from `from` (a plugin; `None`: the player), with
    /// the buttons of `actions` (their titles and the plugin's commands)
    /// after the text. It replaces the message on screen and its buttons,
    /// whoever showed them.
    pub(crate) fn put_osd(
        &mut self,
        text: &str,
        from: Option<u64>,
        actions: Option<(OsdActions, Vec<String>)>,
    ) {
        if self.shell.is_none() {
            return;
        }
        if self.osd_shown.visible {
            self.osd_debug(&OsdEvent::Hide("replaced"));
        }
        let titles = actions
            .as_ref()
            .map_or(&[][..], |(_, titles)| titles.as_slice());
        let title = |i: usize| titles.get(i).cloned().unwrap_or_default();
        self.osd_shown = OsdShown {
            text: text.to_owned(),
            titles: [title(0), title(1)],
            visible: true,
        };
        self.write_osd();
        let ms = if actions.is_some() {
            OSD_ACTION_MS
        } else {
            OSD_MS
        };
        let now = self.core.now_ms();
        self.osd_until = Some(now + ms);
        self.osd_shown_at = now;
        self.osd_clear_at = None;
        self.osd_from = from;
        self.osd_actions = actions.map(|(actions, _)| actions);
        self.osd_debug(&OsdEvent::Show(text));
    }
    /// Write the whole OSD state into the shell, after every change; its
    /// remaining time is the host's.
    pub(crate) fn write_osd(&self) {
        if let Some(shell) = &self.shell {
            for (name, value) in self.osd_shown.values() {
                let _ = shell.set_property(name, value);
            }
        }
    }
    /// The message fades out (`reason` for `VESTRA_DEBUG_OSD`); its buttons
    /// go now, their titles once the fade is over (`OSD_CLEAR_MS`).
    pub(crate) fn hide_osd(&mut self, reason: &str) {
        self.osd_debug(&OsdEvent::Hide(reason));
        self.osd_until = None;
        self.osd_actions = None;
        self.osd_clear_at = Some(self.core.now_ms() + OSD_CLEAR_MS);
        self.osd_shown.visible = false;
        self.write_osd();
    }
    /// Empty both button slots: no button element stays (hidden) in the
    /// accessibility tree.
    pub(crate) fn clear_osd_titles(&mut self) {
        self.osd_clear_at = None;
        self.osd_shown.titles = Default::default();
        self.write_osd();
    }
    /// The buttons of `owner`'s message go (it stopped); the text stays for
    /// its time.
    pub(crate) fn drop_osd_actions(&mut self, owner: u64) {
        if self.osd_actions.as_ref().is_some_and(|a| a.owner == owner) {
            self.osd_debug(&OsdEvent::Hide("its plugin stopped: buttons only"));
            self.osd_actions = None;
            self.clear_osd_titles();
        }
    }
    /// A `VESTRA_DEBUG_OSD` line for the message on screen.
    pub(crate) fn osd_debug(&self, event: &OsdEvent<'_>) {
        if !debug_osd() {
            return;
        }
        let from = self.osd_from.map_or_else(
            || "the player".to_owned(),
            |owner| self.core.plugin_id(owner),
        );
        let actions = self
            .osd_actions
            .as_ref()
            .map_or(0, |actions| actions.commands.len());
        diag!("{}", osd_line(event, &from, actions));
    }
    /// The user pressed button `index` of the message: the message goes and
    /// the plugin's command runs, as if picked from a menu.
    pub(crate) fn osd_action(&mut self, index: usize) {
        let Some(id) = self
            .osd_actions
            .as_ref()
            .and_then(|actions| actions.commands.get(index))
            .cloned()
        else {
            return;
        };
        let owner = self.osd_actions.as_ref().map_or(0, |actions| actions.owner);
        self.hide_osd("action pressed");
        // The message is on the main window's video area.
        self.act(owner, origins::Origin::Main);
        if let Err(error) = self.execute_command(owner, &id) {
            diag!("osd action {id}: {}", error.message);
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
        if id == bar_buttons::ADJUST {
            self.toggle_adjust();
            return;
        }
        if id == palette::PALETTE {
            self.open_palette();
            return;
        }
        if id == ASK_AGAIN {
            crate::capabilities::ask_again_before_plugin_commands(self);
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
impl Core {
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

    /// `VESTRA_DEBUG_OSD`: one line per show and hide, naming the plugin,
    /// its buttons and why the message went.
    #[test]
    fn osd_debug_lines_name_the_plugin_the_buttons_and_the_reason() {
        assert_eq!(
            osd_line(
                &OsdEvent::Show("Clip exported as a.mp4"),
                "vestra.capture",
                2
            ),
            "osd: show from vestra.capture, 2 actions: \"Clip exported as a.mp4\""
        );
        assert_eq!(
            osd_line(&OsdEvent::Hide("hold cap"), "vestra.capture", 1),
            "osd: hide (hold cap) from vestra.capture, 1 action"
        );
        let shown = 1_000;
        assert_eq!(osd_hide_reason(5_000, shown, false, true, false), "time");
        assert_eq!(osd_hide_reason(5_000, shown, true, false, false), "time");
        assert_eq!(
            osd_hide_reason(5_000, shown, true, true, true),
            "pointer idle"
        );
        assert_eq!(
            osd_hide_reason(shown + OSD_HOLD_MS, shown, true, true, false),
            "hold cap"
        );
    }

    /// The OSD state the host writes: the message, its two button slots
    /// and whether it shows.
    #[test]
    fn the_osd_state_is_the_four_shell_properties() {
        let shown = OsdShown {
            text: "Saved".into(),
            titles: ["Show in Folder".into(), String::new()],
            visible: true,
        };
        let values = shown.values();
        let names: Vec<_> = values.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            ["osd-text", "osd-action-1", "osd-action-2", "osd-visible"]
        );
        assert_eq!(values[1].1, SValue::String("Show in Folder".into()));
        assert_eq!(values[2].1, SValue::String("".into()));
        assert_eq!(values[3].1, SValue::Bool(true));
        assert_eq!(OsdShown::default().values()[3].1, SValue::Bool(false));
    }

    #[test]
    fn osd_hold_needs_actions_an_active_pointer_and_ends_by_the_cap() {
        let shown = 1_000;
        // Due at 5 s: held 2 s at a time while the active pointer is on it.
        assert_eq!(osd_hold(5_000, shown, true, true, false), Some(7_000));
        // No actions, no hover, or a resting pointer: it goes.
        assert_eq!(osd_hold(5_000, shown, false, true, false), None);
        assert_eq!(osd_hold(5_000, shown, true, false, false), None);
        assert_eq!(osd_hold(5_000, shown, true, true, true), None);
        // A hover that never ends (the message moved under a still pointer)
        // holds it at most `OSD_HOLD_MS` from when it showed.
        let cap = shown + OSD_HOLD_MS;
        assert_eq!(osd_hold(cap - 500, shown, true, true, false), Some(cap));
        assert_eq!(osd_hold(cap, shown, true, true, false), None);
        assert_eq!(osd_hold(cap + 10_000, shown, true, true, false), None);
        let mut now = shown + OSD_ACTION_MS;
        let mut ticks = 0;
        while let Some(until) = osd_hold(now, shown, true, true, false) {
            assert!(until > now);
            now = until;
            ticks += 1;
        }
        assert_eq!(now, cap);
        assert!(ticks < 20);
    }

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

    /// Every command the player registers itself has a category of
    /// `core_actions::CATEGORIES`, so the command palette and Keyboard
    /// Shortcuts never fall back to "Vestra Player" or the menu path.
    #[test]
    fn every_player_command_has_a_category() {
        let own: Vec<(&str, Option<String>)> = Host::host_declarations()
            .into_iter()
            .map(|(id, declaration, _)| (id, declaration.category))
            .collect();
        let category = |name: &str| Some(name.to_owned());
        assert_eq!(
            own,
            [
                ("fullscreen", category("Window")),
                ("enter-fullscreen", category("Window")),
                ("exit-fullscreen", category("Window")),
                ("quit", category("File")),
                ("sidebar", category("Window")),
                ("palette", category("Help")),
                ("adjust", category("Window")),
                ("ask-again", category("Command Palette")),
            ]
        );
        let own = Host::host_declarations()
            .into_iter()
            .map(|(id, declaration, _)| (id, declaration));
        for (id, declaration) in own.chain(core_actions::declarations(5.)) {
            let key = declaration.category.as_deref().unwrap_or_default();
            assert!(core_actions::CATEGORIES.contains(&key), "{id}: {key:?}");
            assert_eq!(declaration.shown_category(), Some(key), "{id}");
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
        // The short menu (user ruling 2026-11-11): Tools ▸, last of the
        // windows group, with an ellipsis like every item that opens a view.
        assert_eq!(declaration.path, ["Tools", "Command Palette…"]);
        assert_eq!(declaration.shown_title(), "Command Palette…");
        assert_eq!(
            (declaration.order, declaration.group.as_str()),
            (829, "windows")
        );
        assert!(state.enabled);
    }

    /// The short menu (user ruling 2026-11-11): the player's items sit where
    /// doc/product/menus.md puts them. Toggle Fullscreen and the two column
    /// toggles left the menu (Esc, the bottom bar and the palette keep
    /// them); Command Palette… is in Tools ▸ (820); Quit closes the menu in
    /// the settings group with Tools ▸ and Options… (830), no separator
    /// above it; Play/Pause (core_actions.rs) opens the domain group after
    /// Close.
    #[test]
    fn host_menu_items_have_their_short_menu_places() {
        let declarations = Host::host_declarations();
        let place = |id: &str| {
            declarations
                .iter()
                .find(|(each, ..)| *each == id)
                .map(|(_, declaration, _)| {
                    (
                        declaration.path.clone(),
                        declaration.order,
                        declaration.group.clone(),
                    )
                })
                .unwrap()
        };
        assert_eq!(
            place("quit"),
            (vec!["Quit".to_owned()], 990, "settings-main".to_owned())
        );
        assert_eq!(
            place("palette"),
            (
                vec!["Tools".to_owned(), "Command Palette…".to_owned()],
                829,
                "windows".to_owned()
            )
        );
        let in_menu: Vec<&str> = declarations
            .iter()
            .filter(|(_, declaration, _)| !declaration.path.is_empty())
            .map(|(id, ..)| *id)
            .collect();
        assert_eq!(in_menu, ["quit", "palette"]);
        let tools = commands::MENU_ANCHORS
            .iter()
            .find(|(id, ..)| *id == "tools")
            .unwrap();
        assert_eq!(*tools, ("tools", "Tools", 820));
    }

    /// Menu redesign step 3b: Adjustments and Effects (`host/adjust`) and
    /// Sidebar (`host/sidebar`) toggle their columns; no default key,
    /// checkable (the dot), and disabled until a panel is in the column.
    /// User ruling 2026-11-08: the strip button and the palette row share
    /// one title; out of the right-click menu since the short menu (user
    /// ruling 2026-11-11).
    #[test]
    fn column_toggles_are_checkable_host_commands_without_a_key() {
        for (id, title) in [
            (bar_buttons::ADJUST, "Adjustments and Effects"),
            (bar_buttons::SIDEBAR, "Sidebar"),
        ] {
            let (_, declaration, state) = Host::host_declarations()
                .into_iter()
                .find(|(each, ..)| *each == id)
                .unwrap();
            assert!(declaration.bindings.is_empty(), "{id}");
            assert_eq!(state.checked, Some(false), "{id}");
            assert!(!state.enabled, "{id}");
            assert_eq!(declaration.shown_title(), title);
            assert!(declaration.path.is_empty(), "{id}");
        }
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
