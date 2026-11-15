//! The window tick, the tabs of the columns (the sidebar, the adjust
//! column) and the sidebar's floating window.
use super::messages::{UiReport, WindowFacts};
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
        // The title strip, the tab keys and the bar read the interface's
        // copies of the tables (rule R8).
        self.sync_tables();
        self.pip_tick();
        self.menu_tick();
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        let now = self.core.now_ms();
        let fullscreen = shell.window().is_fullscreen();
        // `WindowState.maximized`: never while fullscreen (a window maximized
        // before fullscreen reads maximized again after it). A main window
        // restored maximized reads maximized from the start, before
        // `Placements` maximizes it, so automatic sizing leaves it alone.
        let placing = self.placements.placing(shell.window());
        let maximized = !fullscreen
            && (shell.window().is_maximized() || (placing && self.remembered.window_maximized));
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
            // The approval sheet stays while the user is elsewhere.
            && self.menu_mode != MenuMode::Review
        {
            self.close_menu();
            self.policy.activity(now);
        }
        self.policy.set_fullscreen(fullscreen, now);
        self.policy
            .set_obstructions(self.menu_open, self.core.dialog_busy);
        #[cfg(feature = "producer-audio")]
        let captured = self
            .core
            .backend
            .input
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .is_some_and(|routing| routing.captured);
        #[cfg(not(feature = "producer-audio"))]
        let captured = false;
        {
            let mut input = self.input.borrow_mut();
            input.blocked = self.menu_open || self.core.dialog_busy || captured;
            input.dialog = self.core.dialog_busy;
        }
        if let Err(error) = shell.set_property(
            "input-blocked",
            (self.menu_open || self.core.dialog_busy).into(),
        ) {
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
        let hover = matches!(
            shell.get_property("lead-strip-hover"),
            Ok(slint_interpreter::Value::Bool(true))
        );
        self.lead_strip_tick(now, hover);
        self.review_tick(now);
        self.core.host_command_states(fullscreen);
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
        self.core.backdrop_cover_art();
        let scale = f64::from(shell.window().scale_factor());
        let presentation = &self.core.backend.render.presentation;
        presentation.sample(Instant::now());
        if let Some(log) = self.frame_log.as_mut() {
            let ((mut produced, mut shown), mut size) =
                (self.core.backend.frame.counts(), self.shown_size);
            if let Some(gpu) = self.core.backend.render.gpu() {
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
                    self.core.backend.frame.stages.text(),
                    frame_pacing::log_text(),
                    self.core
                        .backend
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
            self.follow_video_area(&shell, fullscreen);
            return;
        }
        // IA-CORE-03: the normal frame and whether the window is maximized;
        // fullscreen keeps what was under it.
        if !self.core.headless && !placing {
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
        // The pointer's auto-hide below reads the interface's copy of the
        // window settings (plan step S2c(7)).
        self.sync_window_settings();
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
            let playing = self.source["phase"] == "ready" && self.source["paused"] == false;
            // UI-SHELL-09: in fullscreen the pointer hides while playing and
            // idle (3 s without motion or keys, not over the bar or in its
            // band, no menu, dialog or bar lock); FS-BAR-1 decoupled it from
            // the bar. `setCursorAutoHide("playing")`: in a window too, while
            // it has focus.
            // Never while the pointer is at the top edge or over the title strip.
            let idle = self.policy.idle(now) && !self.title_strip.holding();
            let windowed = self.cursor_playing && idle && native.is_some_and(|facts| facts.active);
            shell.set_property(
                "cursor-hidden",
                (playing && ((fullscreen && idle) || windowed)).into(),
            )?;
            Ok(
                json!({"shellGeneration":self.shell_generation,"fullscreen":fullscreen,"maximized":maximized,"logicalWidth":viewport.width,"logicalHeight":viewport.height,"videoArea":{"width":video.0,"height":video.1},"menuOpen":self.menu_open,"dialogOpen":self.core.dialog_busy,"nativeActive":native.map(|facts|facts.active),"nativeUnavailable":self.native_unavailable,"slots":snapshot,"backdrop":self.backdrop_state(),"pictureInPicture":self.pip_state()}),
            )
        })();
        // The slots' containers show their panels (a column opened, a tab
        // switched): before the video area is measured.
        self.sync_panels();
        // The render size from the layout this tick leaves, after the bar,
        // the columns and fullscreen above changed it (read before them, a
        // toggled column reached the renderer a tick later).
        let video = self.follow_video_area(&shell, fullscreen);
        self.answer_video_area(video, now);
        self.bar_tick(&shell, columns_seen);
        match result {
            Ok(facts) => {
                let detached = facts["slots"]["sidebar"]["effective"] == true
                    && facts["slots"]["sidebar"]["detached"] == true;
                self.sync_floating(detached);
                let changed = facts != self.window_facts;
                if changed || self.policy_tabs() != self.tabs_sent {
                    if changed {
                        self.measured_shell = Some(self.shell_generation);
                        self.window_facts = facts.clone();
                    }
                    let report = self.window_facts_report(Some(&shell), facts);
                    // Rule R9: the facts go to the core as a snapshot. Until
                    // plan step S3a it runs on this thread, so the report is
                    // applied here, in place: at the point and in the order
                    // of today's `window.state` broadcast (which the core
                    // sends only when the facts changed, not for the tabs
                    // alone).
                    self.ui_report(UiReport::WindowFacts(Box::new(report)));
                }
            }
            Err(error) => self.broadcast(
                "host.error",
                json!({"code":"window-policy","message":error.to_string()}),
            ),
        }
        // The state file is the core's to write (plan step S2c(7)): its
        // debounced write stays at today's point, the end of this tick.
        self.core.ui_state.save_if_due(now);
    }
    /// The UI's facts value `facts` as the core receives it (rule R9),
    /// with the next `seq` of the UI's one counter. Besides the facts
    /// themselves it carries what later steps read on the core (plan S2c
    /// (5), S2d): the shell's generation, whether the shell has the menu
    /// origin, picture-in-picture as `facts` shows it, and the columns' tab
    /// order and bottom panel (`PanelTabs`, read on the core since plan
    /// step S2c(5)). Also the R13 handoff of a mount batch and of
    /// `layout.set` (`Host::facts_handoff`), and the tabs alone
    /// (`report_tabs`). Without a shell: no generation, no menu origin.
    pub(super) fn window_facts_report(
        &mut self,
        shell: Option<&slint_interpreter::ComponentInstance>,
        facts: Value,
    ) -> WindowFacts {
        self.facts_seq += 1;
        let pip = &facts["pictureInPicture"];
        let tabs = self.policy_tabs();
        self.tabs_sent = tabs.clone();
        WindowFacts {
            seq: self.facts_seq,
            shell: shell.map(|_| self.shell_generation),
            menu_origin: shell.is_some_and(|shell| {
                shell.get_property("menu-origin-x").is_ok()
                    && shell.get_property("menu-origin-y").is_ok()
            }),
            pip_supported: pip["supported"] == true,
            pip_active: pip["active"] == true,
            sidebar_panels: tabs.sidebar,
            adjust_panels: tabs.adjust,
            bottom_panel: tabs.bottom,
            facts,
        }
    }
    /// The panel policy's tabs (`state::PanelTabs`).
    pub(crate) fn policy_tabs(&self) -> state::PanelTabs {
        state::PanelTabs {
            bottom: self.policy.bottom(),
            sidebar: self.policy.sidebar_panels(),
            adjust: self.policy.panels(window_state::Slot::Adjust),
        }
    }
    /// The panel policy's tabs changed outside the window tick (a panel
    /// attached, removed, reordered or rekeyed): the interface reports them
    /// to the core at once (rule R9, plan step S2c(5)), with the facts it
    /// last sent, so the core's copy is never behind the policy when it
    /// judges a request. The facts themselves did not change, so the core
    /// broadcasts nothing.
    pub(crate) fn report_tabs(&mut self) {
        if self.policy_tabs() == self.tabs_sent {
            return;
        }
        let shell = self.shell.as_ref().map(ComponentHandle::clone_strong);
        let facts = self.window_facts.clone();
        let report = self.window_facts_report(shell.as_ref(), facts);
        self.ui_report(UiReport::WindowFacts(Box::new(report)));
    }
    /// Debug builds: the core's copy of the tabs is the policy's wherever
    /// the core reads it (plan step S2c(5): `report_tabs` after every
    /// change). Removed with the in-place delivery (plan S3a).
    #[cfg(debug_assertions)]
    pub(crate) fn check_panel_tabs(&self) {
        debug_assert_eq!(
            self.core.panel_tabs,
            self.policy_tabs(),
            "the core's panel tabs are behind the policy"
        );
    }
    /// A report of the UI reaches the core (rule R3), delivered in place on
    /// this thread until plan step S3a. Plan step S2c(1): the window facts;
    /// S2c(3): the mount reports and the `ui.setMount` result
    /// (host/mounts.rs); S2c(4): the `settings.set` and `settings.show`
    /// results; S2c(5): the tabs with the facts, `layout.set`'s result, a
    /// sheet's confirmation and a confirmation that failed to show, and
    /// where the user acted (`Acted`); S2c(7): the interface's part of the
    /// state file (`Remembered`) and a complete file drop (`Drop`). The
    /// other reports follow in later steps.
    pub(crate) fn ui_report(&mut self, report: UiReport) {
        match report {
            UiReport::WindowFacts(facts) => {
                // A value older than the one applied is skipped (module
                // `messages`: a coalesced snapshot can overtake a handoff).
                // The core broadcasts `window.state` from the facts (R9): its
                // conditions and triggers are evaluated first (`broadcast`);
                // a value with only the tabs changed broadcasts nothing.
                if self.core.apply_facts(*facts) {
                    let facts = self.core.window_facts.clone();
                    self.broadcast("window.state", facts);
                }
            }
            UiReport::MountPreparing(changes) => self.mounts_preparing(changes),
            UiReport::MountApplied(applied) => self.mounts_applied(applied),
            // R13: the handoff first, then the reply.
            UiReport::Answered {
                reply,
                result,
                handoff,
                confirm_failed,
            } => {
                let changed = handoff.is_some_and(|facts| self.core.apply_facts(*facts));
                (self.core.dialog_busy, self.core.confirm_open) = dialog_flags_after(
                    (self.core.dialog_busy, self.core.confirm_open),
                    confirm_failed,
                );
                self.respond(reply, result);
                if changed {
                    let facts = self.core.window_facts.clone();
                    self.broadcast("window.state", facts);
                }
            }
            UiReport::Acted { owner, origin } => self.core.act(owner, origin),
            // Plan step S2c(7): the interface's part of the state file, and
            // a complete file drop.
            UiReport::Remembered { state, at } => self.core.remember(*state, at),
            UiReport::Drop(drop) => self.deliver_drop(drop),
            _ => unreachable!("not reported before plan step S2d"),
        }
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
        self.policy.activity(self.core.now_ms());
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
        if self.remembered.window != Some(frame) {
            self.remembered.window = Some(frame);
            self.remember(self.core.now_ms());
        }
    }
    /// Record whether the main window is maximized and, where the system
    /// gives it (Windows), its normal frame in desktop pixels, also while
    /// maximized, for the next launch.
    fn remember_maximized(&mut self, maximized: bool, window: &slint::Window) {
        let physical = platform::normal_frame(window).or(self.remembered.window_physical);
        let state = &mut self.remembered;
        if state.window_maximized != maximized || state.window_physical != physical {
            state.window_maximized = maximized;
            state.window_physical = physical;
            self.remember(self.core.now_ms());
        }
    }
    /// The interface's part of the state file changed at `now`: the core,
    /// the file's one writer, takes it whole (plan step S2c(7)), in place
    /// at the point where the interface marked the file due before.
    pub(crate) fn remember(&mut self, now: u64) {
        let state = Box::new(self.remembered.clone());
        self.ui_report(UiReport::Remembered { state, at: now });
    }
    /// The slot's remembered state (defaults before anything was saved).
    pub(crate) fn sidebar_slot(&self) -> ui_state::SidebarSlot {
        self.remembered.sidebar_slot.clone().unwrap_or_default()
    }
    /// The adjust column's remembered state (defaults: closed, 317 wide).
    pub(crate) fn adjust_slot(&self) -> ui_state::AdjustSlot {
        self.remembered.adjust_slot.clone().unwrap_or_default()
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
            .filter_map(|owner| self.core.mounts.get(owner).map(|m| m.manifest.clone()))
            .collect::<Vec<_>>();
        let waiting = memory
            .active
            .as_ref()
            .is_some_and(|active| !manifests.contains(active));
        if chosen || !waiting {
            memory.active = self
                .policy
                .active(slot_of)
                .and_then(|owner| self.core.mounts.get(&owner))
                .map(|m| m.manifest.clone());
        }
        let current = manifests.iter().map(String::as_str).collect::<Vec<_>>();
        let now = self.core.now_ms();
        match column {
            Column::Sidebar => {
                let mut slot = self.sidebar_slot();
                slot.visible = memory.visible;
                slot.detached = memory.detached;
                slot.active = memory.active;
                for manifest in &current {
                    slot.remember_among(manifest, &current);
                }
                if self.remembered.sidebar_slot.as_ref() != Some(&slot) {
                    self.remembered.sidebar_slot = Some(slot);
                    self.remember(now);
                }
            }
            Column::Adjust => {
                let mut slot = self.adjust_slot();
                slot.visible = memory.visible;
                slot.active = memory.active;
                for manifest in &current {
                    slot.remember_among(manifest, &current);
                }
                if self.remembered.adjust_slot.as_ref() != Some(&slot) {
                    self.remembered.adjust_slot = Some(slot);
                    self.remember(now);
                }
            }
        }
    }
    /// Hide the whole sidebar (the floating window closed).
    pub(crate) fn close_sidebar(&mut self) {
        if let Some(owner) = self.policy.sidebar_active() {
            let now = self.core.now_ms();
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
    /// plugin is woken once (its activation mounts the panel again). A
    /// running plugin's panel needs nothing here: it compiles on its own
    /// when it is first shown (`sync_panels`).
    fn tab_demand(&mut self, column: Column, active: Option<u64>) {
        let index = column.index();
        let Some(owner) = active else {
            self.columns[index].woken = None;
            return;
        };
        let Some(mount) = self.core.mounts.get(&owner) else {
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
            self.columns[index].woken = None;
        } else if self.columns[index].woken != Some(owner) {
            self.columns[index].woken = Some(owner);
            diag!(
                "{}: tab of {} shown, waking it",
                column.prefix(),
                mount.manifest
            );
            self.panel_shown(plugin);
        }
    }
    /// The current tabs everywhere they matter: the panel keys' owners, the
    /// shell's and the floating window's `<column>-active` (and the floating
    /// window's title), the keys under the adjust column's tabs (the panels
    /// themselves: `sync_panels`). A tab that
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
        // What the host acts on: none while the column waits for its
        // remembered tab at start-up (`current_tab`).
        let current = self.current_tab(column);
        match column {
            Column::Sidebar => self.input.borrow_mut().sidebar = current.map(panel_plugin),
            Column::Adjust => self.input.borrow_mut().adjust = current.map(panel_plugin),
        }
        // Each window indexes its own tab order; a tab it does not have yet
        // (or any more) leaves it as it is until it is rebuilt.
        let active_name = format!("{p}-active");
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
        set_index(shell, &self.columns[index].tabs);
        if column == Column::Sidebar
            && let Some(floating) = &self.floating
        {
            set_index(&floating.instance, &floating.owners);
            if let Some(mount) = active.and_then(|owner| self.core.mounts.get(&owner)) {
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
        if state.active != current {
            let canvas = matches!(shell.get_property("canvas-focused"), Ok(SValue::Bool(true)));
            let more = tabs.len() > 1 || column.always_tabbed();
            if state.active.is_some() && !state.tab_clicked && !canvas && more {
                let _ = shell.invoke("focus-video", &[]);
            }
            state.active = current;
        }
        state.tab_clicked = false;
        // The adjust column's current tab is shown, and needs its panel and
        // its plugin, only while the column is wanted shown.
        let wanted = self
            .policy
            .column_state(policy_column(column))
            .is_some_and(|(preferred, _)| preferred);
        if column == Column::Adjust && !wanted {
            self.columns[index].woken = None;
            return;
        }
        self.tab_demand(column, current);
    }
    /// The keys under a column's tabs (the adjust column, menu redesign
    /// §3.3): each tab's command that opens it (`sidebar_tabs::tab_command`)
    /// with its current key, "" for none. Written only when the binding
    /// table, the commands, the shell or the tabs changed.
    fn tab_keys(&mut self, shell: &slint_interpreter::ComponentInstance, column: Column) {
        self.sync_tables();
        let index = column.index();
        let from = (
            self.input.borrow().table.revision(),
            self.registry.revision(),
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
                let entries = self.registry.entries_of(plugin);
                sidebar_tabs::tab_command(
                    entries
                        .iter()
                        .map(|entry| entry.declaration.home.as_deref()),
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
            .filter_map(|owner| self.core.mounts.get(owner).map(|m| m.title.clone()))
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
            let events = self.core.events.clone();
            window.set_callback(&format!("{p}-tab"), move |args| {
                if let Some(SValue::Number(index)) = args.first()
                    && *index >= 0.
                {
                    let _ = events.send(Event::Tab(generation, column, *index as usize));
                }
                SValue::Void
            })?;
            let events = self.core.events.clone();
            window.set_callback(&format!("{p}-overflow"), move |args| {
                if let [SValue::Number(x), SValue::Number(y)] = args {
                    let _ = events.send(Event::TabOverflow(generation, column, *x, *y));
                }
                SValue::Void
            })?;
        }
        let events = self.core.events.clone();
        window.set_callback("sidebar-detach", move |_| {
            let _ = events.send(Event::SidebarDetach(generation));
            SValue::Void
        })?;
        if has_adjust {
            let events = self.core.events.clone();
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
        match (&self.floating, want) {
            (Some(floating), true) if floating.owners == owners => {
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
                self.core
                    .mounts
                    .get(owner)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no sidebar mount"))
            })
            .collect::<Result<Vec<_>>>()?;
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
        // The window is compiled once a run; the current tab's panel is
        // shown in its container (`ui/panels.rs`) before it opens.
        let instance = composition::floating()?;
        instance.set_property("panel-title", SValue::String(title.as_str().into()))?;
        instance.set_property("sidebar-tabbed", tabbed.into())?;
        instance.set_property("sidebar-titles", self.tab_titles(owners)?)?;
        if let Some(index) = active.and_then(|a| owners.iter().position(|&o| o == a)) {
            instance.set_property("sidebar-active", SValue::Number(index as f64))?;
        }
        self.bind_menu(&instance, generation, false)?;
        self.observe_keys(&instance, generation);
        self.bind_keys(&instance, generation)?;
        view::bind_focus(&instance)?;
        self.bind_tabs(&instance, generation)?;
        let events = self.core.events.clone();
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
        if self.show_panel(&instance, generation, composition::Slot::Sidebar, active) {
            view::settle_layout(instance.window());
        }
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
            generation,
            native,
        });
        // A tab still waiting for its plugin's restart (`panel_restart.rs`).
        self.sync_restart_lines();
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
            self.remembered.sidebar_slot = Some(slot);
            self.remember(self.core.now_ms());
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
impl Core {
    /// Window facts (`UiReport::WindowFacts`, or the handoff of
    /// `UiReport::Answered` and `UiReport::MountApplied`) reach the core's
    /// copy when newer than the last applied (module `messages`), with the
    /// panel tabs. Whether the core's facts value changed: then
    /// `window.state` goes out (after the reply, for a handoff). The
    /// interface sends a snapshot only when its facts or its tabs changed,
    /// and its last facts are the core's, so "changed" is today's
    /// condition for the broadcast.
    pub(crate) fn apply_facts(&mut self, facts: WindowFacts) -> bool {
        if !facts.newer_than(self.window_facts_seq) {
            return false;
        }
        let WindowFacts {
            seq,
            facts,
            sidebar_panels,
            adjust_panels,
            bottom_panel,
            ..
        } = facts;
        self.window_facts_seq = Some(seq);
        self.panel_tabs = state::PanelTabs {
            bottom: bottom_panel,
            sidebar: sidebar_panels,
            adjust: adjust_panels,
        };
        let changed = self.window_facts != facts;
        self.window_facts = facts;
        changed
    }
}

/// The native-dialog flags (`dialog_busy`, `confirm_open`) after a
/// `UiReport::Answered`: only a system confirmation's failure to show
/// (`confirm_failed`) clears them, as `platform::confirm`'s own error did
/// before plan step S2c(5); any other answer leaves them, also an error of
/// another request of the same plugin with the same id.
fn dialog_flags_after(flags: (bool, bool), confirm_failed: bool) -> (bool, bool) {
    if confirm_failed {
        (false, false)
    } else {
        flags
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

    /// Review of S2c(5): `dialogs.confirm` id 7 shows a system dialog,
    /// then the same plugin's `settings.set` id 7 is answered with an
    /// error (the host tolerates a repeated id). That answer is not the
    /// confirmation's: the dialog is still open, so `dialog_busy` and
    /// `confirm_open` stay set (`window.state.dialogOpen`, the `busy`
    /// refusal of a second dialog, `Busy::Dialog`). Only the report marked
    /// `confirm_failed` clears them.
    #[test]
    fn only_the_confirmations_own_failure_clears_the_dialog_flags() {
        let reply = Reply { owner: 3, id: 7 };
        let settings_error = UiReport::Answered {
            reply: reply.clone(),
            result: Err(RpcError::invalid("value does not convert")),
            handoff: None,
            confirm_failed: false,
        };
        let confirm_error = UiReport::Answered {
            reply,
            result: Err(RpcError::new("unavailable", "no dialog")),
            handoff: None,
            confirm_failed: true,
        };
        let open = (true, true);
        for (report, expected) in [(settings_error, open), (confirm_error, (false, false))] {
            let UiReport::Answered { confirm_failed, .. } = report else {
                unreachable!()
            };
            assert_eq!(dialog_flags_after(open, confirm_failed), expected);
        }
        // A file picker open (busy, no confirmation): only the marker clears.
        assert_eq!(dialog_flags_after((true, false), false), (true, false));
    }
}
