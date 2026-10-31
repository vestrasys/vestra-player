//! The window tick, the sidebar's tabs and its floating window.
use super::*;
use crate::playback::renderer::GpuBackend as _;

impl Host {
    pub(crate) fn window_tick(&mut self) {
        self.pip_tick();
        self.menu_tick();
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        let now = self.now_ms();
        let fullscreen = shell.window().is_fullscreen();
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
            self.osd_until = None;
            let _ = shell.set_property("osd-visible", false.into());
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
        // IA-CORE-03: only the normal size is remembered (not fullscreen, not
        // maximized), and the player always starts at it (`RestoreLastState=0`).
        if !fullscreen && !shell.window().is_maximized() && !self.headless {
            self.remember_window(&shell);
        }
        // G-04 / G-01: where the pointer is. A hover counts only while its
        // panel is showing (Slint keeps `has-hover` until the pointer moves).
        let flag = |name: &str| matches!(shell.get_property(name), Ok(SValue::Bool(true)));
        let bottom_hover = flag("bottom-hover") && flag("bottom-visible");
        self.policy
            .set_hover(window_state::Slot::Bottom, bottom_hover);
        self.policy.set_hover(
            window_state::Slot::Sidebar,
            flag("sidebar-hover") && self.sidebar_popped,
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
        if let Some((x, _)) = over_video {
            self.policy.pointer(x, viewport.width, now);
        } else if bottom_hover {
            // The bar lies left of the popped-out sidebar.
            self.policy.pointer(0., viewport.width, now);
        }
        self.policy.tick(now);
        self.sync_tabs(&shell);
        let mut sidebar_seen = None;
        let result = (|| -> Result<Value> {
            let snapshot = self.policy.evaluate(viewport, now)?;
            sidebar_seen = snapshot.sidebar;
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
            if self.sidebar_popped && !popped {
                let _ = shell.invoke("focus-video", &[]);
            }
            self.sidebar_popped = popped;
            let playing =
                self.last_source["phase"] == "ready" && self.last_source["paused"] == false;
            // UI-SHELL-09: in fullscreen the pointer hides while playing and
            // idle (3 s without motion or keys, not over the bar or in its
            // band, no menu, dialog or bar lock); FS-BAR-1 decoupled it from
            // the bar. `setCursorAutoHide("playing")`: in a window too, while
            // it has focus.
            let idle = self.policy.idle(now);
            let windowed =
                self.cursor_playing.get() && idle && native.is_some_and(|facts| facts.active);
            shell.set_property(
                "cursor-hidden",
                (playing && ((fullscreen && idle) || windowed)).into(),
            )?;
            Ok(
                json!({"shellGeneration":self.shell_generation,"fullscreen":fullscreen,"logicalWidth":viewport.width,"logicalHeight":viewport.height,"videoArea":{"width":video.0,"height":video.1},"menuOpen":self.menu_open,"dialogOpen":self.dialog_busy,"nativeActive":native.map(|facts|facts.active),"nativeUnavailable":self.native_unavailable,"slots":snapshot,"backdrop":self.backdrop_state(),"pictureInPicture":self.pip_state()}),
            )
        })();
        self.answer_video_area(video, now);
        self.bar_tick(&shell, sidebar_seen);
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
    /// The slot's remembered state (defaults before anything was saved).
    pub(crate) fn sidebar_slot(&self) -> ui_state::SidebarSlot {
        self.ui_state.state.sidebar_slot.clone().unwrap_or_default()
    }
    /// Keep the sidebar slot's persisted state current: shown, floating, the
    /// current tab and the tab order.
    /// `chosen`: the current tab was just chosen (a click, `layout.set`
    /// `active`), so it is remembered even over one not mounted yet.
    pub(crate) fn remember_sidebar(&mut self, chosen: bool) {
        let Some((visible, detached)) = self.policy.sidebar_state() else {
            return;
        };
        let mut slot = self.sidebar_slot();
        slot.visible = visible;
        slot.detached = detached;
        // A remembered tab whose plugin has not mounted (yet: plugins start
        // one after another) stays remembered, so it is current again once
        // it does.
        let manifests = self
            .policy
            .sidebar_panels()
            .iter()
            .filter_map(|owner| self.mounts.get(owner).map(|m| m.manifest.clone()))
            .collect::<Vec<_>>();
        let waiting = slot
            .active
            .as_ref()
            .is_some_and(|active| !manifests.contains(active));
        if chosen || !waiting {
            slot.active = self
                .policy
                .sidebar_active()
                .and_then(|owner| self.mounts.get(&owner))
                .map(|m| m.manifest.clone());
        }
        for owner in self.policy.sidebar_panels() {
            if let Some(mount) = self.mounts.get(&owner) {
                slot.remember(&mount.manifest);
            }
        }
        if self.ui_state.state.sidebar_slot.as_ref() != Some(&slot) {
            self.ui_state.state.sidebar_slot = Some(slot);
            self.ui_state.touch(self.now_ms());
        }
    }
    /// Hide the whole sidebar (its close button, the floating window closed).
    pub(crate) fn close_sidebar(&mut self) {
        if let Some(owner) = self.policy.sidebar_active() {
            let now = self.now_ms();
            let _ = self
                .policy
                .set_preferred(window_state::Slot::Sidebar, owner, false, now);
            self.remember_sidebar(false);
        }
    }
    /// The sidebar's panels in tab order: the playlist's, other built-in plugins', then as
    /// remembered (`SidebarSlot::order`), new ones last in mount order.
    pub(crate) fn tab_order(&self, mounts: &BTreeMap<u64, Mount>) -> Vec<u64> {
        let slot = self.sidebar_slot();
        let mut tabs = mounts
            .values()
            .filter(|m| m.spec.slot == composition::Slot::Sidebar)
            .map(|m| {
                // The playlist is always first (also as a development load).
                let rank = if m.manifest == "vestra.playlist" {
                    0
                } else if m.builtin {
                    1
                } else {
                    2
                };
                (
                    (rank, slot.position(&m.manifest), m.spec.owner),
                    m.spec.owner,
                )
            })
            .collect::<Vec<_>>();
        tabs.sort();
        tabs.into_iter().map(|(_, owner)| owner).collect()
    }
    /// The current tab everywhere it matters: the panel keys' owner, the
    /// shell's and the floating window's `sidebar-active` (and the floating
    /// window's title). A tab that changed without a click on the strip
    /// (a plugin, the current panel removed) hands the keys back to the
    /// video if they were in the sidebar: the panel that had them is gone.
    pub(crate) fn sync_tabs(&mut self, shell: &slint_interpreter::ComponentInstance) {
        let tabs = self.policy.sidebar_panels();
        let active = self.policy.sidebar_active();
        self.input.borrow_mut().sidebar = active;
        // Each window indexes its own tab order; a tab it does not have yet
        // (or any more) leaves it as it is until it is rebuilt.
        let set_index = |window: &slint_interpreter::ComponentInstance, order: &[u64]| {
            let Some(index) = active.and_then(|owner| order.iter().position(|&o| o == owner))
            else {
                return;
            };
            let index = index as f64;
            if !matches!(window.get_property("sidebar-active"), Ok(SValue::Number(n)) if n == index)
            {
                let _ = window.set_property("sidebar-active", SValue::Number(index));
            }
        };
        set_index(shell, &self.shell_tabs);
        if let Some(floating) = &self.floating {
            set_index(&floating.instance, &floating.owners);
            if let Some(mount) = active.and_then(|owner| self.mounts.get(&owner)) {
                let title = SValue::String(mount.title.as_str().into());
                if floating.instance.get_property("panel-title").ok().as_ref() != Some(&title) {
                    let _ = floating.instance.set_property("panel-title", title);
                }
            }
        }
        if self.sidebar_active != active {
            let canvas = matches!(shell.get_property("canvas-focused"), Ok(SValue::Bool(true)));
            if self.sidebar_active.is_some() && !self.tab_clicked && !canvas && tabs.len() > 1 {
                let _ = shell.invoke("focus-video", &[]);
            }
            self.sidebar_active = active;
        }
        self.tab_clicked = false;
    }
    /// The titles of the sidebar's tabs, in tab order.
    pub(crate) fn tab_titles(&self, owners: &[u64]) -> Result<SValue> {
        let titles = owners
            .iter()
            .filter_map(|owner| self.mounts.get(owner).map(|m| m.title.clone()))
            .collect::<Vec<_>>();
        ui_values::from_json(&json!(titles))
    }
    /// The host's tab strip callbacks of a shell or floating window.
    pub(crate) fn bind_tabs(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        let events = self.events.clone();
        window.set_callback("sidebar-tab", move |args| {
            if let Some(SValue::Number(index)) = args.first()
                && *index >= 0.
            {
                let _ = events.send(Event::SidebarTab(generation, *index as usize));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("sidebar-detach", move |_| {
            let _ = events.send(Event::SidebarDetach(generation));
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("sidebar-close", move |_| {
            let _ = events.send(Event::SidebarClose(generation));
            SValue::Void
        })?;
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
                            self.send(*owner, json!({"event":"ui.error","data":{"code":"ui-prepare","message":format!("ui-prepare: {error:#}")}}));
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
    pub(crate) fn open_floating(&mut self, owners: &[u64]) -> Result<()> {
        let mounts = owners
            .iter()
            .map(|owner| {
                self.mounts
                    .get(owner)
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("no sidebar mount"))
            })
            .collect::<Result<Vec<_>>>()?;
        let specs = mounts.iter().map(|m| m.spec.clone()).collect::<Vec<_>>();
        let active = self.policy.sidebar_active();
        let title = mounts
            .iter()
            .find(|m| Some(m.spec.owner) == active)
            .unwrap_or(&mounts[0])
            .title
            .clone();
        self.next_shell_generation += 1;
        let generation = self.next_shell_generation;
        let instance =
            composition::compile_floating(&specs, &title, self.mount_events.clone(), generation)?;
        for mount in &mounts {
            for (name, value) in self.mount_values(mount) {
                composition::set_global_value(&instance, &mount.spec, &name, value)?;
            }
            composition::set_tabbed(&instance, &mount.spec, mounts.len() > 1)?;
        }
        instance.set_property("sidebar-titles", self.tab_titles(owners)?)?;
        self.bind_menu(&instance, generation, false)?;
        self.observe_keys(&instance, generation);
        self.bind_keys(&instance, generation)?;
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
