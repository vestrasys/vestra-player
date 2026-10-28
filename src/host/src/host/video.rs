//! Video frames on the shell, picture-in-picture, the render size and
//! clicks on the video.
use super::*;

thread_local! {
    /// The UI thread's frame delivery, run by the renderer's wake-up.
    pub(crate) static FRAME_SINK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    /// The UI thread's delivery of GPU frames to picture-in-picture.
    pub(crate) static PIP_GPU_SINK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

impl Host {
    /// GPU rendering: show the renderer's texture in this window (before
    /// `show()`, so the rendering notifier sees the setup). A window that
    /// cannot show it switches the player to software rendering. Either way
    /// the window's draws are counted (`stats().presentedFps`). Returns the
    /// window's reused video texture, if any, for `video_texture` once the
    /// window is shown.
    #[must_use]
    pub(crate) fn attach_gpu(
        &self,
        shell: &slint_interpreter::ComponentInstance,
    ) -> Option<vestra_video_texture::Feed> {
        let Some(gpu) = self.backend.render.gpu() else {
            return self.count_draws(shell);
        };
        let render = self.backend.render.clone();
        let drawn = move |draw: platform::GpuDraw| match draw {
            platform::GpuDraw::Begin => render.presentation.draw_begin(Instant::now()),
            platform::GpuDraw::End { copied } => {
                render.presentation.draw_end(Instant::now(), copied);
            }
        };
        self.backend.render.presentation.start_counting();
        let render = self.backend.render.clone();
        let target = self.video_target.clone();
        let show = move |image: slint::Image| {
            // A generated stream's picture stays (its frames come as pixels).
            if !render.producer()
                && let Some(shell) = target.borrow().upgrade()
            {
                let _ = shell.set_property("video-frame", image.into());
            }
        };
        let render = self.backend.render.clone();
        let failed = move |reason: String| render.fall_back(&reason);
        if let Err(error) = gpu.attach(shell, show, failed, drawn) {
            self.backend
                .render
                .fall_back(&format!("the window cannot show GPU frames ({error})"));
        }
        None
    }
    /// Software rendering: count the window's draws with its rendering
    /// notifier (femtovg); Slint's software renderer has none, so there each
    /// winit redraw counts (without draw times). Where enabled
    /// (`platform::VIDEO_TEXTURE`), the notifier also shows the pictures
    /// through one reused texture (`show_frame`), returned for `video_texture`.
    pub(crate) fn count_draws(
        &self,
        shell: &slint_interpreter::ComponentInstance,
    ) -> Option<vestra_video_texture::Feed> {
        if platform::VIDEO_TEXTURE {
            let render = self.backend.render.clone();
            let drawn = move |draw| match draw {
                vestra_video_texture::Draw::Begin => {
                    let now = Instant::now();
                    frame_pacing::draw_began();
                    render.presentation.draw_begin(now);
                }
                vestra_video_texture::Draw::End => {
                    render.presentation.draw_end(Instant::now(), false);
                }
            };
            let target = self.video_target.clone();
            let show = move |image: slint::Image| {
                if let Some(shell) = target.borrow().upgrade() {
                    let _ = shell.set_property("video-frame", image.into());
                }
            };
            if let Ok(feed) = vestra_video_texture::attach(shell, show, drawn) {
                self.backend.render.presentation.start_counting();
                return Some(feed);
            }
        }
        let render = self.backend.render.clone();
        let notifier = move |state, _: &slint::GraphicsAPI| match state {
            slint::RenderingState::BeforeRendering => {
                let now = Instant::now();
                frame_pacing::draw_began();
                render.presentation.draw_begin(now);
            }
            slint::RenderingState::AfterRendering => {
                render.presentation.draw_end(Instant::now(), false);
            }
            _ => (),
        };
        if shell.window().set_rendering_notifier(notifier).is_err() {
            use slint::winit_030::{EventResult, WinitWindowAccessor, winit::event::WindowEvent};
            let render = self.backend.render.clone();
            shell.window().on_winit_window_event(move |_, event| {
                if matches!(event, WindowEvent::RedrawRequested) {
                    render.presentation.draw_end(Instant::now(), false);
                }
                EventResult::Propagate
            });
        }
        self.backend.render.presentation.start_counting();
        None
    }
    /// A new shell took the main window's place inside the same native
    /// window (`prepare_mount_tick`): what was attached to the window stays
    /// (its rendering notifier and reused texture, its native window id and
    /// title bar) and now serves `shell`; what followed the old shell
    /// follows the new one.
    pub(crate) fn retarget_shell(&self, shell: &slint_interpreter::ComponentInstance) {
        self.video_target.replace(shell.as_weak());
        if let Some(gpu) = self.backend.render.gpu() {
            gpu.retarget(shell);
        }
        frame_pacing::follow(shell);
    }
    /// Set the newest rendered frame on the video area (no copy: the renderer
    /// drew into this buffer).
    pub(crate) fn show_frame(&mut self) {
        // Paced (Windows): one new picture per draw; a second one within the
        // same refresh comes again right after the draw.
        if !frame_pacing::accept_picture() {
            self.backend.frame.miss();
            return;
        }
        let Some(frame) = self.backend.frame.take() else {
            return;
        };
        let Some(window) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        self.shown_size = Some((frame.buffer.width(), frame.buffer.height()));
        let (width, height) = (frame.buffer.width(), frame.buffer.height());
        if self.pip.feeding() && self.pip_shape((width, height)) {
            self.pip
                .push(width as usize, height as usize, frame.buffer.as_bytes());
        }
        let set = Instant::now();
        // Through the reused texture when the window has one (uploaded at
        // the next draw); otherwise as a new image (femtovg then makes a new
        // texture for it, Slint's software renderer composes it).
        let texture = self
            .video_texture
            .borrow()
            .as_ref()
            .and_then(|feed| feed.put(frame.buffer.clone()));
        let image = slint::Image::from_rgb8(frame.buffer);
        self.last_image = Some(image.clone());
        self.last_frame = Some(set);
        match texture {
            Some(texture) => {
                if let Some(texture) = texture {
                    let _ = window.set_property("video-frame", texture.into());
                }
                window.window().request_redraw();
            }
            None => {
                let _ = window.set_property("video-frame", image.into());
            }
        }
        self.backend.render.presentation.picture_set(set.elapsed());
        frame_pacing::picture_set();
        // With GPU rendering these are a generated stream's frames (or its
        // cleared canvas): the next GPU frame sets its texture again.
        if let Some(gpu) = self.backend.render.gpu() {
            gpu.reshow();
        }
    }
    /// What `window.setPictureInPicture` and the PiP window depend on.
    pub(crate) fn pip_facts(&self) -> pip::Facts {
        pip::Facts {
            window: !self.headless && self.native_window.is_some(),
            supported: !self.headless && platform::pip_supported(),
            fullscreen: self
                .shell
                .as_ref()
                .is_some_and(|s| s.window().is_fullscreen()),
            video: !self.video_absent,
        }
    }
    /// `window.setPictureInPicture { on }` (R-W5): answered once the system
    /// window has opened or closed. From fullscreen the player leaves
    /// fullscreen first.
    pub(crate) fn set_picture_in_picture(
        &mut self,
        reply: &Reply,
        params: &Value,
    ) -> std::result::Result<Option<Value>, RpcError> {
        let on = params["on"]
            .as_bool()
            .ok_or_else(|| RpcError::invalid("on requires a boolean"))?;
        let facts = self.pip_facts();
        let now = self.now_ms();
        let answer = self
            .pip
            .request(reply.clone(), on, &facts, now, platform::PipWindow::new)?;
        if on
            && answer.is_none()
            && facts.fullscreen
            && let Some(shell) = &self.shell
        {
            shell.window().set_fullscreen(false);
        }
        Ok(answer)
    }
    /// Each UI tick (R-H8): keep the PiP source layer on the video area,
    /// report playback to the PiP window, run what its buttons ask, and
    /// show the main window's "playing in picture-in-picture" note.
    pub(crate) fn pip_tick(&mut self) {
        // Switch the renderers to the picture's shape before any frame goes
        // to PiP (frames of the old shape are not pushed).
        self.apply_render_size();
        self.pip_gpu_hook();
        if self.pip.needs_frame() {
            if self.pip_gpu_frames() {
                // Paused playback renders nothing new: start from the newest.
                self.pip_gpu_frame();
            } else if let Some(still) = self.last_image.as_ref().and_then(slint::Image::to_rgb8)
                && self.pip_shape((still.width(), still.height()))
            {
                let (width, height) = (still.width() as usize, still.height() as usize);
                self.pip.push(width, height, still.as_bytes());
            }
        }
        let place = self
            .shell
            .as_ref()
            .zip(self.native_window)
            .map(|(shell, id)| {
                view::settle_layout(shell.window());
                let number = |name: &str| match shell.get_property(name) {
                    Ok(SValue::Number(n)) => n,
                    _ => 0.,
                };
                let area = platform::Rect {
                    x: number("video-x"),
                    y: number("video-y"),
                    width: number("video-width"),
                    height: number("video-height"),
                };
                (id, area)
            });
        let source = &self.last_source;
        let seconds = |key: &str| source[key].as_f64().filter(|n| n.is_finite());
        let playback = pip::Playback {
            paused: source["paused"] == true,
            position: seconds("position"),
            duration: seconds("duration").filter(|_| source["capabilities"]["seek"] == true),
            rate: seconds("rate").unwrap_or(1.),
        };
        let facts = self.pip_facts();
        let now = self.now_ms();
        if std::env::var_os("VESTRA_PIP_LOG").is_some()
            && let Some((id, area)) = place
            && let Some(line) = self.pip.describe(id)
        {
            let scale = self
                .shell
                .as_ref()
                .map_or(0., |s| s.window().scale_factor());
            let line = format!(
                "pip: video area ({:.1},{:.1} {:.1}x{:.1}) pt, slint scale {scale} | render area {:?} sent {:?} picture {:?} | {line}",
                area.x,
                area.y,
                area.width,
                area.height,
                self.render_area,
                self.render_sent,
                self.backend
                    .target
                    .picture()
                    .map(|(w, h)| (w.round(), h.round())),
            );
            if self.pip_log.as_deref() != Some(line.as_str()) {
                diag!("{line}");
                self.pip_log = Some(line);
            }
        }
        for effect in self.pip.tick(&facts, place, &playback, now) {
            match effect {
                pip::Effect::Respond(reply, result) => self.respond(reply, result),
                pip::Effect::Action(action, step) => {
                    if let Err(error) = self.run_core_action_with(action, None, step) {
                        diag!("pip: {action}: {}", error.message);
                    }
                }
            }
        }
        if let Some(shell) = &self.shell {
            let active = self.pip.active();
            if shell.get_property("picture-in-picture").ok() != Some(active.into()) {
                let _ = shell.set_property("picture-in-picture", active.into());
            }
        }
    }
    /// GPU rendering: once, have the render thread's frame callback wake the
    /// UI thread for each new frame while PiP wants frames (coalesced; a
    /// wake-up that finds the host busy is dropped, the next frame comes).
    pub(crate) fn pip_gpu_hook(&mut self) {
        let (wanted, hooked) = &mut self.pip_gpu;
        wanted.store(self.pip.feeding(), Ordering::Release);
        let Some(gpu) = self.backend.render.gpu().filter(|_| !*hooked) else {
            return;
        };
        *hooked = true;
        let wanted = wanted.clone();
        let queued = Arc::new(AtomicBool::new(false));
        gpu.on_frame(move || {
            if !wanted.load(Ordering::Acquire) || queued.swap(true, Ordering::AcqRel) {
                return;
            }
            let queued = queued.clone();
            let _ = slint::invoke_from_event_loop(move || {
                queued.store(false, Ordering::Release);
                PIP_GPU_SINK.with(|sink| {
                    if let Some(deliver) = &*sink.borrow() {
                        deliver();
                    }
                });
            });
        });
    }
    /// PiP shows the GPU renderer's frames: GPU rendering, and no generated
    /// stream (whose pixel frames go through `show_frame` like software ones).
    pub(crate) fn pip_gpu_frames(&self) -> bool {
        self.backend.render.gpu().is_some() && !self.backend.render.producer()
    }
    /// The GPU renderer's newest frame to PiP, without a copy (each frame
    /// once). The renderer does not draw into a surface the system still
    /// shows, so the PiP window never sees a frame being redrawn.
    pub(crate) fn pip_gpu_frame(&mut self) {
        if !self.pip.feeding() || !self.pip_gpu_frames() {
            return;
        }
        let Some(gpu) = self.backend.render.gpu() else {
            return;
        };
        if let Some((sequence, surface)) = gpu.latest()
            && gpu.size().is_some_and(|size| self.pip_shape(size))
        {
            self.pip.push_surface(sequence, &surface);
        }
    }
    /// The size both renderers draw at: the video area, or while PiP wants
    /// mpv's frames the picture's own shape inside it (no bars, R-H8; the
    /// PiP window takes the frame's shape). Sent when it changes.
    pub(crate) fn apply_render_size(&mut self) {
        let Some(area) = self.render_area else {
            return;
        };
        let picture = self.backend.target.picture();
        let size = match picture {
            Some(picture) if self.pip.feeding() && !self.backend.render.producer() => {
                render_target::fit(area, picture)
            }
            _ => area,
        };
        if self.render_sent != Some(size) {
            self.render_sent = Some(size);
            self.backend.target.set_area(size);
            if let Some(gpu) = self.backend.render.gpu() {
                gpu.set_size(size);
            }
        }
    }
    /// A frame of `size` pixels may go to PiP: it has the shape the renderer
    /// was last asked for (a frame from before the switch to the picture's
    /// shape would size the PiP window wrongly). Generated streams' frames
    /// always may.
    pub(crate) fn pip_shape(&self, size: (u32, u32)) -> bool {
        self.backend.render.producer()
            || self
                .render_sent
                .is_none_or(|sent| render_target::same_shape(size, sent))
    }
    /// R-H11: a pending left press on the video becomes `Mouse:LeftClick`
    /// on release when nothing makes it wait (`video_click`), else once the
    /// double-click interval passes without a second press. It runs here,
    /// not through the event queue, so it costs no further loop turn.
    pub(crate) fn click_tick(&mut self, pointer: (f64, f64)) {
        let Some(click) = self.input.borrow().click else {
            return;
        };
        let input = bindings::Input {
            mods: click.mods,
            name: "Mouse:LeftClick".into(),
        };
        let immediate = video_click::immediate(&self.input.borrow().table, &self.commands, &input);
        let verdict = click.verdict(Instant::now(), pointer, double_click_interval(), immediate);
        if verdict == video_click::Verdict::Wait {
            return;
        }
        self.input.borrow_mut().click = None;
        let target = {
            let shared = self.input.borrow();
            (!shared.blocked)
                .then(|| shared.table.resolve(&input, None).cloned())
                .flatten()
        };
        if verdict == video_click::Verdict::Fire
            && click.generation == self.shell_generation
            && let Some(target) = target
        {
            // A click also focuses the video and starts a window drag, so a
            // click whose command cannot run now (play/pause with nothing
            // open) does nothing, without the reason a key would show.
            if matches!(
                self.commands.resolve_execution(target.owner, &target.id),
                Err(commands::CommandError::Disabled { .. })
            ) {
                input_trace::mark(&format!(
                    "click dropped: {}/{} is disabled",
                    target.owner, target.id
                ));
                return;
            }
            input_trace::mark(&format!(
                "click fired ({}) → {}/{}",
                if immediate {
                    "on release"
                } else {
                    "after the double-click interval"
                },
                target.owner,
                target.id
            ));
            self.handle(Event::Binding(click.generation, target.owner, target.id));
        }
    }
}
