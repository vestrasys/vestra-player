//! The window tick, the tabs of the columns (the sidebar, the adjust
//! column) and the sidebar's floating window.
use super::mounts::policy_column;
use super::*;
use crate::playback::renderer::GpuBackend as _;
use crate::ui::sidebar_tabs::Column;

/// What a column remembers across launches (`ui_state`'s `sidebarSlot`,
/// `adjustSlot`).
pub(crate) struct ColumnMemory {
    pub(crate) visible: bool,
    pub(crate) detached: bool,
    pub(crate) active: Option<String>,
    pub(crate) order: Vec<String>,
}

impl Host {
    pub(crate) fn window_tick(&mut self) {
        self.pip_tick();
        self.menu_tick();
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        let now = self.now_ms();
        let fullscreen = shell.window().is_fullscreen();
        // `WindowState.maximized`: never while fullscreen (a window maximized
        // before fullscreen reads maximized again after it). A main window
        // restored maximized reads maximized from the start, before
        // `Placements` maximizes it, so automatic sizing leaves it alone.
        let placing = self.placements.placing(shell.window());
        let maximized = !fullscreen
            && (shell.window().is_maximized() || (placing && self.ui_state.state.window_maximized));
        if self.native_window.is_none()
            && let Some(before) = &self.native_before
        {
            match platform::associate_new(before) {
                Ok(id) => {
                    self.native_window = Some(id);
                    self.native_before = None;
                    self.native_unavailable = None;
                }
                Err(error) => self.native_unavailable = Some(error.to_string()),
            }
        }
        let native = self
            .native_window
            .and_then(|id| platform::screen_for(id).ok());
        if native.is_some_and(|facts| !facts.active)
            && self.menu_open
            && self.menu_generation == self.shell_generation
        {
            self.close_menu();
            self.policy.activity(now);
        }
        self.policy.set_fullscreen(fullscreen, now);
        self.policy
            .set_obstructions(self.menu_open, self.dialog_busy);
        #[cfg(feature = "producer-audio")]
        let captured = self
            .backend
            .input
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|routing| routing.captured);
        #[cfg(not(feature = "producer-audio"))]
        let captured = false;
        self.input.borrow_mut().blocked = self.menu_open || self.dialog_busy || captured;
        if let Err(error) =
            shell.set_property("input-blocked", (self.menu_open || self.dialog_busy).into())
        {
            diag!("host.error physical-input bridge: {error}");
        }
        if self.osd_until.is_some_and(|until| now >= until) {
            // A message with actions does not fade while the active pointer
            // is on it (at most `OSD_HOLD_MS`); it goes 2 s after the
            // pointer leaves or rests.
            let hover = matches!(
                shell.get_property("osd-hover"),
                Ok(slint_interpreter::Value::Bool(true))
            );
            let (actions, idle) = (self.osd_actions.is_some(), self.policy.idle(now));
            match osd_hold(now, self.osd_shown_at, actions, hover, idle) {
                Some(until) => self.osd_until = Some(until),
                None => {
                    let reason = osd_hide_reason(now, self.osd_shown_at, actions, hover, idle);
                    self.hide_osd(reason);
                }
            }
        }
        if self.osd_clear_at.is_some_and(|at| now >= at) {
            self.clear_osd_titles();
        }
        self.host_command_states(fullscreen);
        // The sidebar and plugin panels show and hide `if` elements.
        view::settle_layout(shell.window());
        let number = |name: &str| match shell.get_property(name) {
            Ok(slint_interpreter::Value::Number(n)) => n,
            _ => 0.,
        };
        let viewport = window_state::Viewport {
            width: number("logical-width"),
            height: number("logical-height"),
        };
        let video = (number("video-width"), number("video-height"));
        self.backdrop_cover_art();
        let scale = f64::from(shell.window().scale_factor());
        if let Some(area) = self
            .sizing
            .observe(video, scale, fullscreen, Instant::now())
        {
            self.render_area = Some(area);
        }
        self.apply_render_size();
        let presentation = &self.backend.render.presentation;
        presentation.sample(Instant::now());
        if let Some(log) = self.frame_log.as_mut() {
            let ((mut produced, mut shown), mut size) =
                (self.backend.frame.counts(), self.shown_size);
            if let Some(gpu) = self.backend.render.gpu() {
                (produced, shown) = (produced + gpu.frames(), shown + gpu.shown());
                size = gpu.size().or(size);
            }
            let (draws, presented) = presentation.counts();
            let counts = render_target::LogCounts {
                produced,
                shown,
                draws,
                presented,
            };
            let refresh = || platform::window_refresh(shell.window());
            if let Some(line) = log.line(counts, presentation, refresh, size, Instant::now()) {
                diag!(
                    "{line}{}{}{}{}",
                    self.backend.frame.stages.text(),
                    frame_pacing::log_text(),
                    self.backend
                        .render
                        .gpu()
                        .map_or_else(String::new, |gpu| gpu.log_text()),
                    self.video_texture
                        .borrow()
                        .as_ref()
                        .map_or_else(String::new, |feed| feed.take_upload_text())
                );
            }
        }
        self.apply_on_top();
        self.click_tick((number("pointer-x"), number("pointer-y")));
        if self.measured_shell != Some(self.shell_generation)
            && (viewport.width <= 0. || viewport.height <= 0.)
        {
            return;
        }
        // IA-CORE-03: the normal frame and whether the window is maximized;
        // fullscreen keeps what was under it.
        if !self.headless && !placing {
            let minimized = shell.window().is_minimized();
            match record(fullscreen, maximized, minimized) {
                Record::Normal => {
                    self.remember_window(&shell);
                    self.remember_maximized(false, shell.window());
                }
                Record::Maximized => self.remember_maximized(true, shell.window()),
                Record::Nothing => {}
            }
        }
        // G-04 / G-01: where the pointer is. A hover counts only while its
        // panel is showing (Slint keeps `has-hover` until the pointer moves).
        let flag = |name: &str| matches!(shell.get_property(name), Ok(SValue::Bool(true)));
        let bottom_hover = flag("bottom-hover") && flag("bottom-visible");
        self.policy
            .set_hover(window_state::Slot::Bottom, bottom_hover);
        self.policy.set_hover(
            window_state::Slot::Sidebar,
            flag("sidebar-hover") && self.columns[Column::Sidebar.index()].popped,
        );
        self.policy.set_hover(
            window_state::Slot::Adjust,
            flag("adjust-hover") && self.columns[Column::Adjust.index()].popped,
        );
        // FS-BAR-1: the fullscreen bar shows only while the pointer is in
        // its band (the bar's laid-out height, also while it is hidden).
        let over_video = flag("video-hover").then(|| (number("pointer-x"), number("pointer-y")));
        self.policy.bottom_band(
            bottom_hover,
            over_video,
            viewport,
            number("bottom-band-height"),
            now,
        );
        // The title strip's reveal zone, from the window's own pointer (the
        // winit hook), wherever it is: over the video, the strip; in
        // fullscreen only left of the popped-out columns (G-19), as the
        // strip ends there.
        let at_edge = self
            .input
            .borrow()
            .pointer
            .is_some_and(|(generation, point)| {
                generation == self.shell_generation
                    && self
                        .policy
                        .at_title_edge(point.x, point.y, scale, viewport.width)
            });
        if let Some((x, _)) = over_video {
            // The top-right corner reveals the title strip, not the sidebar.
            if !at_edge {
                self.policy.pointer(x, viewport.width, now);
            }
        } else if bottom_hover {
            // The bar lies left of the popped-out sidebar.
            self.policy.pointer(0., viewport.width, now);
        }
        self.policy.tick(now);
        self.title_strip_tick(&shell, fullscreen, at_edge, now);
        self.sync_tabs(&shell);
        let mut columns_seen = (None, None);
        let result = (|| -> Result<Value> {
            let snapshot = self.policy.evaluate(viewport, now)?;
            columns_seen = (snapshot.sidebar, snapshot.adjust);
            shell.set_property("host-fullscreen", fullscreen.into())?;
            shell.set_property(
                "menu-open",
                (self.menu_open && self.menu_generation == self.shell_generation).into(),
            )?;
            let bottom = snapshot.bottom.is_some_and(|s| s.effective);
            shell.set_property("bottom-visible", bottom.into())?;
            // Slint 1.18 drops the focus of an element that becomes invisible
            // (#11079), and then no key reaches the bindings: hiding the bar
            // gives the focus back to the video, as collapsing the sidebar does.
            if self.bottom_shown && !bottom && !flag("canvas-focused") {
                let _ = shell.invoke("focus-video", &[]);
            }
            self.bottom_shown = bottom;
            let sidebar = snapshot.sidebar.filter(|s| !s.detached);
            shell.set_property(
                "sidebar-visible",
                sidebar.is_some_and(|s| s.effective && !s.overlay).into(),
            )?;
            // G-01: the fullscreen overlay exists while attached in
            // fullscreen, and slides out while effective.
            shell.set_property("sidebar-overlay", sidebar.is_some_and(|s| s.overlay).into())?;
            let popped = sidebar.is_some_and(|s| s.overlay && s.effective);
            shell.set_property("sidebar-popped", popped.into())?;
            let state = &mut self.columns[Column::Sidebar.index()];
            if state.popped && !popped {
                let _ = shell.invoke("focus-video", &[]);
            }
            state.popped = popped;
            // The adjust column: docked, over the video of a narrow window,
            // or in fullscreen an overlay that exists collapsed too, so it
            // can slide out.
            let adjust = snapshot.adjust;
            let docked = adjust.is_some_and(|a| a.effective && !a.overlay);
            shell.set_property("adjust-visible", docked.into())?;
            shell.set_property(
                "adjust-overlay",
                adjust
                    .is_some_and(|a| a.overlay && (fullscreen || a.effective))
                    .into(),
            )?;
            let popped = adjust.is_some_and(|a| a.overlay && a.effective);
            shell.set_property("adjust-popped", popped.into())?;
            let state = &mut self.columns[Column::Adjust.index()];
            // Hidden, its panels' focus goes; the keys go back to the video
            // (Slint #11079, as for the bar and the sidebar).
            if state.shown && !(popped || docked) && !flag("canvas-focused") {
                let _ = shell.invoke("focus-video", &[]);
            }
            state.popped = popped;
            state.shown = popped || docked;
            let playing =
                self.last_source["phase"] == "ready" && self.last_source["paused"] == false;
            // UI-SHELL-09: in fullscreen the pointer hides while playing and
            // idle (3 s without motion or keys, not over the bar or in its
            // band, no menu, dialog or bar lock); FS-BAR-1 decoupled it from
            // the bar. `setCursorAutoHide("playing")`: in a window too, while
            // it has focus.
            // Never while the pointer is at the top edge or over the title strip.
            let idle = self.policy.idle(now) && !self.title_strip.holding();
            let windowed =
                self.cursor_playing.get() && idle && native.is_some_and(|facts| facts.active);
            shell.set_property(
                "cursor-hidden",
                (playing && ((fullscreen && idle) || windowed)).into(),
            )?;
            Ok(
                json!({"shellGeneration":self.shell_generation,"fullscreen":fullscreen,"maximized":maximized,"logicalWidth":viewport.width,"logicalHeight":viewport.height,"videoArea":{"width":video.0,"height":video.1},"menuOpen":self.menu_open,"dialogOpen":self.dialog_busy,"nativeActive":native.map(|facts|facts.active),"nativeUnavailable":self.native_unavailable,"slots":snapshot,"backdrop":self.backdrop_state(),"pictureInPicture":self.pip_state()}),
            )
        })();
        self.answer_video_area(video, now);
        self.bar_tick(&shell, columns_seen);
        match result {
            Ok(facts) => {
                let detached = facts["slots"]["sidebar"]["effective"] == true
                    && facts["slots"]["sidebar"]["detached"] == true;
                self.sync_floating(detached);
                if facts != self.window_facts {
                    self.measured_shell = Some(self.shell_generation);
                    self.window_facts = facts.clone();
                    self.broadcast("window.state", facts);
                }
            }
            Err(error) => self.broadcast(
                "host.error",
                json!({"code":"window-policy","message":error.to_string()}),
            ),
        }
        self.ui_state.save_if_due(now);
    }
    /// The fullscreen title strip (`title-strip.slint`): it applies to the
    /// fullscreen main window where the system shows no title bar there
    /// (`platform::FULLSCREEN_TITLE_STRIP`), not in picture-in-picture; it
    /// shows while the pointer is at the top edge (`at_edge`, the window's
    /// pointer; not while a menu is open) or over it
    /// (`window_state::TitleStrip`). Properties are written only when they
    /// change, and the tooltip keys only as it comes out: nothing per
    /// pointer move.
    fn title_strip_tick(
        &mut self,
        shell: &slint_interpreter::ComponentInstance,
        fullscreen: bool,
        at_edge: bool,
        now: u64,
    ) {
        let flag = |name: &str| matches!(shell.get_property(name), Ok(SValue::Bool(true)));
        let set = |name: &str, value: SValue| {
            if shell.get_property(name).ok().as_ref() != Some(&value)
                && let Err(error) = shell.set_property(name, value)
            {
                diag!("host.error title strip {name}: {error}");
            }
        };
        let eligible =
            platform::FULLSCREEN_TITLE_STRIP && fullscreen && !flag("picture-in-picture");
        let before = flag("title-strip-shown");
        let shown = self.title_strip.update(
            eligible,
            at_edge && !self.menu_open,
            flag("title-strip-hover"),
            now,
        );
        if shown && !before {
            // Exit Fullscreen's own key, else Toggle Fullscreen's (Esc).
            let (leave, close) = {
                let input = self.input.borrow();
                (
                    input
                        .table
                        .label(bindings::HOST, "exit-fullscreen")
                        .or_else(|| input.table.label(bindings::HOST, "fullscreen")),
                    input.table.label(bindings::HOST, "quit"),
                )
            };
            set(
                "title-strip-exit-key",
                SValue::String(leave.unwrap_or_default().into()),
            );
            set(
                "title-strip-close-key",
                SValue::String(close.unwrap_or_default().into()),
            );
        }
        set("title-strip-enabled", eligible.into());
        set("title-strip-shown", shown.into());
    }
    /// A title strip button: Minimize, Exit Fullscreen (the
    /// `exit-fullscreen` command) and Close (the window close button: Quit).
    pub(crate) fn title_strip_action(&mut self, action: &str) {
        self.policy.activity(self.now_ms());
        match action {
            "minimize" => {
                if let Some(shell) = &self.shell {
                    platform::minimize_window(shell.window());
                }
            }
            "exit-fullscreen" => self.host_command("exit-fullscreen"),
            "close" => self.host_command("quit"),
            _ => {}
        }
    }
    /// Record the main window frame (logical px) for the next launch.
    pub(crate) fn remember_window(&mut self, shell: &slint_interpreter::ComponentInstance) {
        let window = shell.window();
        let scale = window.scale_factor() as f64;
        let (size, position) = (window.size(), window.position());
        if size.width == 0 || size.height == 0 || scale <= 0. {
            return;
        }
        let frame = ui_state::Frame {
            x: position.x as f64 / scale,
            y: position.y as f64 / scale,
            width: size.width as f64 / scale,
            height: size.height as f64 / scale,
        };
        if self.ui_state.state.window != Some(frame) {
            self.ui_state.state.window = Some(frame);
            self.ui_state.touch(self.now_ms());
        }
    }
    /// Record whether the main window is maximized and, where the system
    /// gives it (Windows), its normal frame in desktop pixels, also while
    /// maximized, for the next launch.
    fn remember_maximized(&mut self, maximized: bool, window: &slint::Window) {
        let physical = platform::normal_frame(window).or(self.ui_state.state.window_physical);
        let state = &mut self.ui_state.state;
        if state.window_maximized != maximized || state.window_physical != physical {
            state.window_maximized = maximized;
            state.window_physical = physical;
            self.ui_state.touch(self.now_ms());
        }
    }
    /// The slot's remembered state (defaults before anything was saved).
    pub(crate) fn sidebar_slot(&self) -> ui_state::SidebarSlot {
        self.ui_state.state.sidebar_slot.clone().unwrap_or_default()
    }
    /// The adjust column's remembered state (defaults: closed, 317 wide).
    pub(crate) fn adjust_slot(&self) -> ui_state::AdjustSlot {
        self.ui_state.state.adjust_slot.clone().unwrap_or_default()
    }
    /// What a column remembers across launches: shown, floating (the
    /// sidebar only), its current tab and its tab order.
    pub(crate) fn column_memory(&self, column: Column) -> ColumnMemory {
        match column {
            Column::Sidebar => {
                let slot = self.sidebar_slot();
                ColumnMemory {
                    visible: slot.visible,
                    detached: slot.detached,
                    active: slot.active,
                    order: slot.order,
                }
            }
            Column::Adjust => {
                let slot = self.adjust_slot();
                ColumnMemory {
                    visible: slot.visible,
                    detached: false,
                    active: slot.active,
                    order: slot.order,
                }
            }
        }
    }
    /// Keep the sidebar slot's persisted state current: shown, floating, the
    /// current tab and the tab order (`remember_column`).
    pub(crate) fn remember_sidebar(&mut self, chosen: bool) {
        self.remember_column(Column::Sidebar, chosen);
    }
    /// Keep a column's persisted state current: shown, floating (the
    /// sidebar), the current tab and the tab order. `chosen`: the current
    /// tab was just chosen (a click, `layout.set` `active`), so it is
    /// remembered even over one not mounted yet.
    pub(crate) fn remember_column(&mut self, column: Column, chosen: bool) {
        let slot_of = policy_column(column);
        let Some((visible, detached)) = self.policy.column_state(slot_of) else {
            return;
        };
        let mut memory = self.column_memory(column);
        memory.visible = visible;
        memory.detached = detached;
        // A remembered tab whose plugin has not mounted (yet: plugins start
        // one after another) stays remembered, so it is current again once
        // it does.
        let manifests = self
            .policy
            .panels(slot_of)
            .iter()
            .filter_map(|owner| self.mounts.get(owner).map(|m| m.manifest.clone()))
            .collect::<Vec<_>>();
        let waiting = memory
            .active
            .as_ref()
            .is_some_and(|active| !manifests.contains(active));
        if chosen || !waiting {
            memory.active = self
                .policy
                .active(slot_of)
                .and_then(|owner| self.mounts.get(&owner))
                .map(|m| m.manifest.clone());
        }
        let current = manifests.iter().map(String::as_str).collect::<Vec<_>>();
        let now = self.now_ms();
        match column {
            Column::Sidebar => {
                let mut slot = self.sidebar_slot();
                slot.visible = memory.visible;
                slot.detached = memory.detached;
                slot.active = memory.active;
                for manifest in &current {
                    slot.remember_among(manifest, &current);
                }
                if self.ui_state.state.sidebar_slot.as_ref() != Some(&slot) {
                    self.ui_state.state.sidebar_slot = Some(slot);
                    self.ui_state.touch(now);
                }
            }
            Column::Adjust => {
                let mut slot = self.adjust_slot();
                slot.visible = memory.visible;
                slot.active = memory.active;
                for manifest in &current {
                    slot.remember_among(manifest, &current);
                }
                if self.ui_state.state.adjust_slot.as_ref() != Some(&slot) {
                    self.ui_state.state.adjust_slot = Some(slot);
                    self.ui_state.touch(now);
                }
            }
        }
    }
    /// Hide the whole sidebar (the floating window closed).
    pub(crate) fn close_sidebar(&mut self) {
        if let Some(owner) = self.policy.sidebar_active() {
            let now = self.now_ms();
            let _ = self
                .policy
                .set_preferred(window_state::Slot::Sidebar, owner, false, now);
            self.remember_sidebar(false);
        }
    }
    /// A column's panels in tab order (`sidebar_tabs::order`): its first
    /// tabs (the playlist; Picture, Audio, Subtitles, Playback), the other
    /// built-in plugins', then as remembered, new ones last in mount order.
    pub(crate) fn tab_order(&self, column: Column, mounts: &BTreeMap<u64, Mount>) -> Vec<u64> {
        let tabs = mounts
            .values()
            .filter(|m| m.spec.slot == column.slot())
            .map(|m| sidebar_tabs::Tab {
                owner: m.spec.owner,
                manifest: &m.manifest,
                builtin: m.builtin,
            })
            .collect::<Vec<_>>();
        sidebar_tabs::order(column, &tabs, &self.column_memory(column).order)
    }
    /// What a column's current tab needs (sidebar-tabs.md): a placeholder's
    /// plugin is woken once (its activation mounts the panel again); a
    /// running plugin's panel that is not in the shell is compiled into it
    /// (`prepare_mount_tick`), unless that just failed for this tab. The
    /// adjust column's first opening compiles its panel this way (menu
    /// redesign, open question 1: on demand).
    fn tab_demand(&mut self, column: Column, active: Option<u64>) {
        let index = column.index();
        let Some(owner) = active else {
            self.columns[index].woken = None;
            return;
        };
        let Some(mount) = self.mounts.get(&owner) else {
            return;
        };
        if mount.spec.slot != column.slot() {
            return;
        }
        // Its running plugin is kept up while the tab is current (declared
        // work, `Busy::ShownTab`), also when it woke under a new owner after
        // the tab was shown: registered here, every tick, not only on a change.
        let plugin = mount.plugin;
        if !mount.recorded {
            self.tab_work(plugin);
        }
        let Some(mount) = self.mounts.get(&owner) else {
            return;
        };
        let pending = self.mount_queue.iter().any(|c| c.key == owner)
            || self.preparing.as_ref().is_some_and(|(_, c)| c.key == owner);
        // Under its own key, or its kept branch after its plugin restarted.
        let compiled = self.shell_branch(owner, true).is_some();
        match sidebar_tabs::need(mount.recorded, compiled, pending) {
            sidebar_tabs::Need::Wake => {
                if self.columns[index].woken != Some(owner) {
                    self.columns[index].woken = Some(owner);
                    diag!(
                        "{}: tab of {} shown, waking it",
                        column.prefix(),
                        mount.manifest
                    );
                    self.panel_shown(plugin);
                }
            }
            sidebar_tabs::Need::Compile => {
                self.columns[index].woken = None;
                if self.columns[index].refresh_failed != Some(owner)
                    && !self.tabs_failed.contains(&owner)
                {
                    self.shell_refresh = true;
                }
            }
            sidebar_tabs::Need::Nothing => {
                self.columns[index].woken = None;
                if compiled {
                    self.columns[index].refresh_failed = None;
                }
            }
        }
    }
    /// The sidebar's compiled tabs, in tab order, by the owner each branch was
    /// compiled under (`shell_compiled`; a woken plugin's tab keeps its old
    /// branch until its new mount): the floating window compiles the same
    /// ones, so a wake does not reopen it.
    pub(crate) fn compiled_tabs(&self) -> Vec<u64> {
        self.columns[Column::Sidebar.index()]
            .tabs
            .iter()
            .filter_map(|owner| self.shell_compiled.get(owner).copied())
            .collect()
    }
    /// The current tabs everywhere they matter: the panel keys' owners, the
    /// shell's and the floating window's `<column>-active` (and the floating
    /// window's title), the keys under the adjust column's tabs. A tab that
    /// changed without a click on the strip (a plugin, the current panel
    /// removed) hands the keys back to the video if they were in the column:
    /// the panel that had them is gone.
    pub(crate) fn sync_tabs(&mut self, shell: &slint_interpreter::ComponentInstance) {
        for column in Column::ALL {
            self.sync_column(shell, column);
        }
    }
    fn sync_column(&mut self, shell: &slint_interpreter::ComponentInstance, column: Column) {
        let index = column.index();
        let p = column.prefix();
        let tabs = self.policy.panels(policy_column(column));
        let active = self.policy.active(policy_column(column));
        match column {
            Column::Sidebar => self.input.borrow_mut().sidebar = active.map(panel_plugin),
            Column::Adjust => self.input.borrow_mut().adjust = active.map(panel_plugin),
        }
        // Each window indexes its own tab order; a tab it does not have yet
        // (or any more) leaves it as it is until it is rebuilt.
        let active_name = format!("{p}-active");
        let shown_name = format!("{p}-shown");
        let set_index = |window: &slint_interpreter::ComponentInstance, order: &[u64]| {
            let Some(index) = active.and_then(|owner| order.iter().position(|&o| o == owner))
            else {
                return;
            };
            let index = index as f64;
            if !matches!(window.get_property(&active_name), Ok(SValue::Number(n)) if n == index) {
                let _ = window.set_property(&active_name, SValue::Number(index));
            }
        };
        let set_shown = |window: &slint_interpreter::ComponentInstance, owner: i64| {
            let owner = owner as f64;
            if !matches!(window.get_property(&shown_name), Ok(SValue::Number(n)) if n == owner) {
                let _ = window.set_property(&shown_name, SValue::Number(owner));
            }
        };
        set_index(shell, &self.columns[index].tabs);
        set_shown(shell, sidebar_tabs::shown(active, &self.shell_compiled));
        if column == Column::Sidebar
            && let Some(floating) = &self.floating
        {
            set_index(&floating.instance, &floating.owners);
            // Its branches are the shell's (`compiled_tabs`), by the same owners.
            let shown = sidebar_tabs::shown(active, &self.shell_compiled);
            let shown = if floating.compiled.iter().any(|&alias| alias as i64 == shown) {
                shown
            } else {
                -1
            };
            set_shown(&floating.instance, shown);
            if let Some(mount) = active.and_then(|owner| self.mounts.get(&owner)) {
                let title = SValue::String(mount.title.as_str().into());
                if floating.instance.get_property("panel-title").ok().as_ref() != Some(&title) {
                    let _ = floating.instance.set_property("panel-title", title);
                }
            }
        }
        if column == Column::Adjust {
            self.tab_keys(shell, column);
        }
        let state = &mut self.columns[index];
        if state.active != active {
            let canvas = matches!(shell.get_property("canvas-focused"), Ok(SValue::Bool(true)));
            let more = tabs.len() > 1 || column.always_tabbed();
            if state.active.is_some() && !state.tab_clicked && !canvas && more {
                let _ = shell.invoke("focus-video", &[]);
            }
            state.active = active;
        }
        state.tab_clicked = false;
        // The adjust column's current tab is shown, and needs its panel, its
        // plugin and its compile, only while the column is wanted shown
        // (opened once, its panels stay compiled).
        let wanted = self
            .policy
            .column_state(policy_column(column))
            .is_some_and(|(preferred, _)| preferred);
        if column == Column::Adjust && !wanted {
            self.columns[index].woken = None;
            return;
        }
        if column == Column::Adjust && !self.columns[index].opened {
            self.columns[index].opened = true;
            diag!("adjust: first opened, its panels compile now");
        }
        self.tab_demand(column, active);
    }
    /// The keys under a column's tabs (the adjust column, menu redesign
    /// §3.3): each tab's command that opens it (`sidebar_tabs::tab_command`)
    /// with its current key, "" for none. Written only when the binding
    /// table, the commands, the shell or the tabs changed.
    fn tab_keys(&mut self, shell: &slint_interpreter::ComponentInstance, column: Column) {
        let index = column.index();
        let from = (
            self.input.borrow().table.revision(),
            self.commands.revision(),
            self.shell_generation,
            self.columns[index].tabs.clone(),
        );
        if self.columns[index].keys_from.as_ref() == Some(&from) {
            return;
        }
        let keys = self.columns[index]
            .tabs
            .iter()
            .map(|&owner| {
                let plugin = panel_plugin(owner);
                let entries = self.commands.entries_of(plugin);
                sidebar_tabs::tab_command(
                    entries
                        .iter()
                        .map(|(entry, ..)| entry.declaration.home.as_deref()),
                )
                .and_then(|id| self.input.borrow().table.label(plugin, id))
                .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        if let Ok(value) = ui_values::from_json(&json!(keys)) {
            let _ = shell.set_property(&format!("{}-keys", column.prefix()), value);
        }
        self.columns[index].keys_from = Some(from);
    }
    /// The titles of a column's tabs, in tab order.
    pub(crate) fn tab_titles(&self, owners: &[u64]) -> Result<SValue> {
        let titles = owners
            .iter()
            .filter_map(|owner| self.mounts.get(owner).map(|m| m.title.clone()))
            .collect::<Vec<_>>();
        ui_values::from_json(&json!(titles))
    }
    /// The host's tab strip callbacks of a shell or floating window (the
    /// adjust column's where the window has it: the shell).
    pub(crate) fn bind_tabs(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        let has_adjust = window
            .definition()
            .callbacks()
            .any(|name| name == "adjust-tab");
        for column in Column::ALL {
            if column == Column::Adjust && !has_adjust {
                continue;
            }
            let p = column.prefix();
            let events = self.events.clone();
            window.set_callback(&format!("{p}-tab"), move |args| {
                if let Some(SValue::Number(index)) = args.first()
                    && *index >= 0.
                {
                    let _ = events.send(Event::Tab(generation, column, *index as usize));
                }
                SValue::Void
            })?;
            let events = self.events.clone();
            window.set_callback(&format!("{p}-overflow"), move |args| {
                if let [SValue::Number(x), SValue::Number(y)] = args {
                    let _ = events.send(Event::TabOverflow(generation, column, *x, *y));
                }
                SValue::Void
            })?;
        }
        let events = self.events.clone();
        window.set_callback("sidebar-detach", move |_| {
            let _ = events.send(Event::SidebarDetach(generation));
            SValue::Void
        })?;
        if has_adjust {
            let events = self.events.clone();
            window.set_callback("adjust-dismiss", move |_| {
                let _ = events.send(Event::AdjustDismiss(generation));
                SValue::Void
            })?;
        }
        Ok(())
    }
    /// Open or close the floating window so it matches the policy; a window
    /// with other panels than the sidebar's now opens again.
    pub(crate) fn sync_floating(&mut self, want: bool) {
        let owners = self.policy.sidebar_panels();
        let compiled = self.compiled_tabs();
        match (&self.floating, want) {
            (Some(floating), true)
                if floating.owners == owners && floating.compiled == compiled =>
            {
                self.remember_floating();
            }
            (None, true) if !owners.is_empty() => {
                if let Err(error) = self.open_floating(&owners) {
                    diag!("floating panel: {error:#}");
                    if let Some(&owner) = owners.first() {
                        for owner in &owners {
                            self.send(panel_plugin(*owner), json!({"event":"ui.error","data":{"code":"ui-prepare","message":format!("ui-prepare: {error:#}")}}));
                        }
                        let _ = self.policy.set_detached(owner, false);
                        self.remember_sidebar(false);
                    }
                }
            }
            (Some(_), _) => self.close_floating(),
            _ => {}
        }
    }
    /// The floating window with every tab in its strip and the shell's
    /// compiled panels (sidebar-tabs.md); it opens again when they change.
    pub(crate) fn open_floating(&mut self, owners: &[u64]) -> Result<()> {
        let tabs = owners
            .iter()
            .map(|owner| {
                self.mounts
                    .get(owner)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no sidebar mount"))
            })
            .collect::<Result<Vec<_>>>()?;
        let compiled = self.compiled_tabs();
        // Each under the owner its shell branch was compiled under.
        let mounts = tabs
            .iter()
            .filter_map(|m| {
                let alias = *self.shell_compiled.get(&m.spec.owner)?;
                let mut mount = m.clone();
                mount.spec.owner = alias;
                Some(mount)
            })
            .collect::<Vec<_>>();
        let specs = mounts.iter().map(|m| m.spec.clone()).collect::<Vec<_>>();
        let active = self.policy.sidebar_active();
        let title = tabs
            .iter()
            .find(|m| Some(m.spec.owner) == active)
            .unwrap_or(&tabs[0])
            .title
            .clone();
        let tabbed = tabs.len() > 1;
        self.next_shell_generation += 1;
        let generation = self.next_shell_generation;
        let instance = composition::compile_floating(
            &specs,
            &title,
            tabbed,
            self.mount_events.clone(),
            generation,
        )?;
        // A value a panel cannot take is reported, never a failure of the
        // whole window (`report_value_failures`).
        let mut failures = Vec::new();
        for mount in &mounts {
            for (name, value) in self.mount_values(mount) {
                if let Err(error) =
                    composition::set_global_value(&instance, &mount.spec, &name, value)
                {
                    failures.push((mount.spec.owner, format!("{name}: {error:#}")));
                }
            }
            composition::set_tabbed(&instance, &mount.spec, tabbed)?;
        }
        self.report_value_failures(failures);
        // A tab still waiting for its plugin's restart (`panel_restart.rs`).
        for tab in &tabs {
            if let (Some(line), Some(&alias)) = (
                self.restarts.text_of(tab.spec.owner),
                self.shell_compiled.get(&tab.spec.owner),
            ) {
                let property = super::panel_restart::restarting_property(alias);
                instance.set_property(&property, SValue::String(line.into()))?;
            }
        }
        instance.set_property("sidebar-titles", self.tab_titles(owners)?)?;
        self.bind_menu(&instance, generation, false)?;
        self.observe_keys(&instance, generation);
        self.bind_keys(&instance, generation)?;
        view::bind_focus(&instance)?;
        self.bind_tabs(&instance, generation)?;
        let events = self.events.clone();
        instance.window().on_close_requested(move || {
            let _ = events.send(Event::FloatingClosed(generation));
            slint::CloseRequestResponse::KeepWindowShown
        });
        let frame = self.sidebar_slot().floating.unwrap_or(ui_state::Frame {
            x: 438.,
            y: 100.,
            width: 300.,
            height: 408.,
        });
        instance.window().set_size(slint::LogicalSize::new(
            frame.width.max(300.) as f32,
            frame.height.max(250.) as f32,
        ));
        self.theme_view(&instance);
        let before = platform::capture_windows().ok();
        self.placements.show_at(
            &instance,
            slint::LogicalPosition::new(frame.x as f32, frame.y as f32),
        )?;
        platform::shown(&instance);
        let native = before.and_then(|before| platform::associate_new(&before).ok());
        self.floating = Some(Floating {
            instance,
            owners: owners.to_vec(),
            compiled,
            generation,
            native,
        });
        Ok(())
    }
    pub(crate) fn remember_floating(&mut self) {
        let Some(floating) = &self.floating else {
            return;
        };
        let window = floating.instance.window();
        let scale = window.scale_factor() as f64;
        let (size, position) = (window.size(), window.position());
        if size.width == 0 || scale <= 0. {
            return;
        }
        let frame = ui_state::Frame {
            x: position.x as f64 / scale,
            y: position.y as f64 / scale,
            width: size.width as f64 / scale,
            height: size.height as f64 / scale,
        };
        let mut slot = self.sidebar_slot();
        if slot.floating != Some(frame) {
            slot.floating = Some(frame);
            self.ui_state.state.sidebar_slot = Some(slot);
            self.ui_state.touch(self.now_ms());
        }
    }
    pub(crate) fn close_floating(&mut self) {
        if let Some(floating) = self.floating.take() {
            if self.menu_open && self.menu_generation == floating.generation {
                self.menu_open = false;
                self.menu_path.clear();
                self.menu_scroll.clear();
                self.menu_aim.clear();
                self.menu_mode = MenuMode::Main;
            }
            let _ = floating.instance.hide();
        }
    }
    /// Quitting: every window but the main one goes away now, before the
    /// plugins and the backend are torn down (a shown Slint window keeps
    /// itself alive until hidden, so dropping it is not enough).
    pub(crate) fn close_secondary_windows(&mut self) {
        self.close_floating();
        for view in std::mem::take(&mut self.views).into_values() {
            let _ = view.instance.hide();
        }
        if let Some(window) = self.settings.window() {
            let _ = window.hide();
        }
    }
    /// Shell or floating window generation still alive.
    pub(crate) fn window_generation_live(&self, generation: u64) -> bool {
        generation == self.shell_generation
            || self
                .floating
                .as_ref()
                .is_some_and(|f| f.generation == generation)
    }
    pub(crate) fn window_for(
        &self,
        generation: u64,
    ) -> Option<&slint_interpreter::ComponentInstance> {
        if generation == self.shell_generation {
            self.shell.as_ref()
        } else {
            self.floating
                .as_ref()
                .filter(|f| f.generation == generation)
                .map(|f| &f.instance)
        }
    }
}

/// What a tick records of the main window for the next launch (IA-CORE-03).
#[derive(Debug, PartialEq, Eq)]
enum Record {
    /// Its frame, and that it is not maximized.
    Normal,
    /// That it is maximized; the frame stays the normal one from before.
    Maximized,
    /// Nothing: fullscreen keeps what was under it (the player never starts
    /// in fullscreen, as Chrome, VLC and mpv), and a minimized window has no
    /// frame of its own (Windows reports it at -32000, -32000).
    Nothing,
}

/// `maximized` as the window reports it (Slint keeps it through a
/// minimize; the tick passes false in fullscreen).
fn record(fullscreen: bool, maximized: bool, minimized: bool) -> Record {
    if fullscreen || minimized {
        Record::Nothing
    } else if maximized {
        Record::Maximized
    } else {
        Record::Normal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Closed in fullscreen or minimized, the player opens as the window was
    /// before; maximized, it opens maximized over the normal frame.
    #[test]
    fn the_window_records_its_normal_frame_and_maximized_never_fullscreen() {
        assert_eq!(record(false, false, false), Record::Normal);
        assert_eq!(record(false, true, false), Record::Maximized);
        for maximized in [false, true] {
            assert_eq!(record(true, maximized, false), Record::Nothing);
            assert_eq!(record(false, maximized, true), Record::Nothing);
            assert_eq!(record(true, maximized, true), Record::Nothing);
        }
    }
}
