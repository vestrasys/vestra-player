//! Video frames on the shell, picture-in-picture, the render size and
//! clicks on the video.
use super::messages::{UiCommand, UiReport, WindowOp};
use super::*;
use crate::playback::render_mode::Reason;
use crate::playback::renderer::GpuBackend as _;

thread_local! {
    /// The UI thread's frame delivery, run by the renderer's wake-up.
    pub(crate) static FRAME_SINK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    /// The UI thread's delivery of GPU frames to picture-in-picture.
    pub(crate) static PIP_GPU_SINK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
    /// The UI thread's `Host::resized`, run after the main window's resize.
    pub(crate) static RESIZE_SINK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
}

/// The main window was resized (its winit hook): `Host::resized` once Slint
/// has handled the event, coalesced (one queued at a time).
pub(crate) fn queue_resized() {
    static QUEUED: AtomicBool = AtomicBool::new(false);
    if QUEUED.swap(true, Ordering::AcqRel) {
        return;
    }
    let queued = slint::invoke_from_event_loop(|| {
        QUEUED.store(false, Ordering::Release);
        RESIZE_SINK.with(|sink| {
            if let Some(resized) = &*sink.borrow() {
                resized();
            }
        });
    });
    // No event loop (any more): the next resize may try again.
    if queued.is_err() {
        QUEUED.store(false, Ordering::Release);
    }
}

/// `VESTRA_TEST_NO_VIDEO_IMAGE=1` (diagnostics): frames are rendered,
/// uploaded and counted, but never set on the video area, so the window's
/// draw cost can be measured without the video in it.
fn video_image_hidden() -> bool {
    static HIDDEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HIDDEN.get_or_init(|| std::env::var("VESTRA_TEST_NO_VIDEO_IMAGE").as_deref() == Ok("1"))
}

/// An event of the main window's winit event hook (`bind_window_events`):
/// with `counting` (a window without a rendering notifier and without the
/// pacing hook's count, `Host::count_draws`), each `RedrawRequested` the
/// window dispatches is one draw. The hook runs after the frame pacing's
/// (`frame_pacing::Handler`), so a redraw held for a vertical blank never
/// reaches it. Returns whether the event counted.
pub(crate) fn count_winit_draw(
    counting: bool,
    event: &slint::winit_030::winit::event::WindowEvent,
    presentation: &crate::playback::render_target::Presentation,
) -> bool {
    use slint::winit_030::winit::event::WindowEvent;
    let draw = counting && matches!(event, WindowEvent::RedrawRequested);
    if draw {
        presentation.draw_end(Instant::now(), false);
    }
    draw
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
        let Some(gpu) = self.core.backend.render.gpu() else {
            return self.count_draws(shell);
        };
        self.winit_draws.set(false);
        let render = self.core.backend.render.clone();
        let drawn = move |draw: platform::GpuDraw| match draw {
            platform::GpuDraw::Begin => {
                let now = Instant::now();
                frame_pacing::draw_began();
                render.presentation.draw_begin(now);
            }
            platform::GpuDraw::End { copied } => {
                frame_pacing::draw_ended();
                render.presentation.draw_end(Instant::now(), copied);
            }
        };
        self.core.backend.render.presentation.start_counting();
        let render = self.core.backend.render.clone();
        let target = self.video_target.clone();
        let notifier = platform::window_notifiers();
        // Without a notifier the window sets each frame's image between
        // draws; paced, it takes one only after the previous one was drawn,
        // and the image it sets owes the draw.
        let paced = !notifier && frame_pacing::gpu_frames_paced();
        let show = move |image: slint::Image| {
            // A generated stream's picture stays (its frames come as pixels).
            if !render.producer() {
                if !video_image_hidden()
                    && let Some(shell) = target.borrow().upgrade()
                {
                    let _ = shell.set_property("video-frame", image.into());
                }
                if !notifier {
                    // Each image is a new frame; the next draw presents it.
                    render.presentation.picture_set(Duration::ZERO);
                }
                if paced {
                    frame_pacing::picture_set();
                }
            }
        };
        if !notifier {
            self.count_draws_without_notifier();
        }
        platform::gate_gpu_frames(
            paced.then(|| Box::new(frame_pacing::accept_picture) as Box<dyn Fn() -> bool>),
        );
        let render = self.core.backend.render.clone();
        let failed = move |reason: String| render.fall_back(Reason::WindowFallback(reason));
        if let Err(error) = gpu.attach(shell, show, failed, drawn) {
            self.core
                .backend
                .render
                .fall_back(Reason::WindowFallback(format!(
                    "the window cannot show GPU frames ({error})"
                )));
        }
        None
    }
    /// Software rendering: count the window's draws with its rendering
    /// notifier (femtovg); Slint's software renderer has none, so there each
    /// winit redraw the window dispatches counts (without draw times), in the
    /// window's one event hook (`bind_window_events`, `winit_draws`). Where
    /// enabled
    /// (`platform::VIDEO_TEXTURE`), the notifier also shows the pictures
    /// through one reused texture (`show_frame`), returned for `video_texture`.
    pub(crate) fn count_draws(
        &self,
        shell: &slint_interpreter::ComponentInstance,
    ) -> Option<vestra_video_texture::Feed> {
        self.winit_draws.set(false);
        if !platform::window_notifiers() {
            // wgpu windows (Windows): no notifier, the texture is filled in
            // `show_frame` itself.
            self.count_draws_without_notifier();
            return platform::direct_video_feed();
        }
        if platform::VIDEO_TEXTURE {
            let render = self.core.backend.render.clone();
            let drawn = move |draw| match draw {
                vestra_video_texture::Draw::Begin => {
                    let now = Instant::now();
                    frame_pacing::draw_began();
                    render.presentation.draw_begin(now);
                }
                vestra_video_texture::Draw::End => {
                    frame_pacing::draw_ended();
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
                self.core.backend.render.presentation.start_counting();
                return Some(feed);
            }
        }
        let render = self.core.backend.render.clone();
        let notifier = move |state, _: &slint::GraphicsAPI| match state {
            slint::RenderingState::BeforeRendering => {
                let now = Instant::now();
                frame_pacing::draw_began();
                render.presentation.draw_begin(now);
            }
            slint::RenderingState::AfterRendering => {
                frame_pacing::draw_ended();
                render.presentation.draw_end(Instant::now(), false);
            }
            _ => (),
        };
        if shell.window().set_rendering_notifier(notifier).is_err() {
            // Not a hook of its own: the window has one winit event hook, and
            // `bind_window_events` sets it after this (before 0.5.15 a hook
            // set here was replaced there, and `presented` stayed 0).
            frame_pacing::on_draw(None);
            self.winit_draws.set(true);
        }
        self.core.backend.render.presentation.start_counting();
        None
    }
    /// Count the main window's draws through the pacing hook, where the
    /// window has no rendering notifier (`platform::window_notifiers`): no
    /// draw times then, only the counts (`stats().presentedFps`).
    fn count_draws_without_notifier(&self) {
        let render = self.core.backend.render.clone();
        frame_pacing::on_draw(Some(Box::new(move || {
            render.presentation.draw_end(Instant::now(), false);
        })));
        self.core.backend.render.presentation.start_counting();
    }
    /// Set the newest rendered frame on the video area (no copy: the renderer
    /// drew into this buffer).
    pub(crate) fn show_frame(&mut self) {
        // Paced (Windows): one new picture per draw; a second one within the
        // same refresh comes again right after the draw.
        if !frame_pacing::accept_picture() {
            self.core.backend.frame.miss();
            return;
        }
        let Some(frame) = self.core.backend.frame.take() else {
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
        // The core keeps the display awake from it (plan step S2f(2)).
        self.ui_report(messages::UiReport::FrameShown(set));
        match texture {
            Some(texture) => {
                // The reused texture is filled in place: set its image only
                // when the video area shows another one (a new texture, a
                // rebuilt shell, a cleared picture). Slint 1.18.1 never finds
                // two wgpu texture images equal, so setting the same one again
                // still counted as a change and redid everything that depends
                // on the picture each frame.
                if let Some(texture) = texture
                    && !video_image_hidden()
                    && !matches!(
                        window.get_property("video-frame"),
                        Ok(slint_interpreter::Value::Image(shown))
                            if platform::same_video_image(&shown, &texture)
                    )
                {
                    let _ = window.set_property("video-frame", texture.into());
                }
                window.window().request_redraw();
            }
            None => {
                if !video_image_hidden() {
                    let _ = window.set_property("video-frame", image.into());
                }
            }
        }
        self.core
            .backend
            .render
            .presentation
            .picture_set(set.elapsed());
        frame_pacing::picture_set();
        // With GPU rendering these are a generated stream's frames (or its
        // cleared canvas): the next GPU frame sets its texture again.
        if let Some(gpu) = self.core.backend.render.gpu() {
            gpu.reshow();
        }
    }
    /// What `window.setPictureInPicture` and the PiP window depend on.
    pub(crate) fn pip_facts(&self) -> pip::Facts {
        pip::Facts {
            window: !self.core.headless && self.native_window.is_some(),
            supported: !self.core.headless && platform::pip_supported(),
            fullscreen: self
                .shell
                .as_ref()
                .is_some_and(|s| s.window().is_fullscreen()),
            video: !self.video_absent,
        }
    }
    /// `window.setPictureInPicture { on }` (R-W5): answered once the system
    /// window has opened or closed. From fullscreen the player leaves
    /// fullscreen first. Plan step S2d: the core checks `on` and sends
    /// `WindowOp::Pip`; the interface's PiP state machine carries it out
    /// (`pip_command`) and answers with `UiReport::Answered`, at once or
    /// from `pip_tick`.
    pub(crate) fn set_picture_in_picture(
        &mut self,
        reply: &Reply,
        params: &Value,
    ) -> std::result::Result<Option<Value>, RpcError> {
        let on = params["on"]
            .as_bool()
            .ok_or_else(|| RpcError::invalid("on requires a boolean"))?;
        self.ui_command(UiCommand::Window(WindowOp::Pip {
            reply: reply.clone(),
            on,
        }));
        Ok(None)
    }
    /// The interface's side of `WindowOp::Pip` (plan step S2d): an answer
    /// the state machine gives at once (a refusal, or nothing to change) is
    /// reported now; otherwise `pip_tick` reports it when the system window
    /// has opened or closed.
    pub(crate) fn pip_command(&mut self, reply: Reply, on: bool) {
        let facts = self.pip_facts();
        let now = self.core.now_ms();
        let answer = self
            .pip
            .request(reply.clone(), on, &facts, now, platform::PipWindow::new);
        if on
            && matches!(answer, Ok(None))
            && facts.fullscreen
            && let Some(shell) = &self.shell
        {
            shell.window().set_fullscreen(false);
        }
        match answer {
            Ok(None) => {}
            Ok(Some(value)) => self.pip_answered(reply, Ok(value)),
            Err(error) => self.pip_answered(reply, Err(error)),
        }
    }
    /// A `window.setPictureInPicture` result of the interface reaches the
    /// core (`UiReport::Answered`, no handoff: `WindowOp::Pip`).
    fn pip_answered(&mut self, reply: Reply, result: std::result::Result<Value, RpcError>) {
        self.ui_report(UiReport::Answered {
            reply,
            result,
            handoff: None,
            confirm_failed: false,
        });
    }
    /// Each UI tick (R-H8): keep the PiP source layer on the video area,
    /// report playback to the PiP window, run what its buttons ask, and
    /// show the main window's "playing in picture-in-picture" note.
    pub(crate) fn pip_tick(&mut self) {
        // Switch the renderers to the picture's shape before any frame goes
        // to PiP (frames of the old shape are not pushed).
        self.apply_render_size();
        self.pip_gpu_hook();
        self.gpu_pacing_hook();
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
        let source = &self.source;
        let seconds = |key: &str| source[key].as_f64().filter(|n| n.is_finite());
        let playback = pip::Playback {
            paused: source["paused"] == true,
            position: seconds("position"),
            duration: seconds("duration").filter(|_| source["capabilities"]["seek"] == true),
            rate: seconds("rate").unwrap_or(1.),
        };
        let facts = self.pip_facts();
        let now = self.core.now_ms();
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
                self.core
                    .backend
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
                pip::Effect::Respond(reply, result) => self.pip_answered(reply, result),
                pip::Effect::Action(action, step) => {
                    if let Err(error) = self.run_core_action_with(action, None, step) {
                        diag!("pip: {action}: {}", error.message);
                    }
                    // What the core's action asked of the interface (plan
                    // step S2f(3)).
                    self.drain_outbox();
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
    /// GPU rendering where the host paces the main window (`frame_pacing`,
    /// Windows): once, have each new frame owe that window a draw at the next
    /// vertical blank, as a new software picture does (`show_frame`), instead
    /// of waiting for Slint's timer to ask (coalesced: one wake-up of the UI
    /// thread at a time). A generated stream's pictures are owed by
    /// `show_frame` itself. Not for windows without a notifier when GPU
    /// frames are paced (`frame_pacing::gpu_frames_paced`): there the image
    /// set between draws owes the draw (`attach_gpu`), so a vertical blank
    /// can no longer fall between a frame's image and the draw it owes.
    pub(crate) fn gpu_pacing_hook(&mut self) {
        if !frame_pacing::wanted()
            || (!platform::window_notifiers() && frame_pacing::gpu_frames_paced())
        {
            return;
        }
        // No GPU renderer (any more, after a fallback): a later one is hooked
        // again.
        let Some(gpu) = self.core.backend.render.gpu() else {
            self.gpu_paced = false;
            return;
        };
        if self.gpu_paced {
            return;
        }
        self.gpu_paced = true;
        let (render, queued) = (
            self.core.backend.render.clone(),
            Arc::new(AtomicBool::new(false)),
        );
        gpu.on_frame(move || {
            if queued.swap(true, Ordering::AcqRel) {
                return;
            }
            let (render, queued) = (render.clone(), queued.clone());
            let _ = slint::invoke_from_event_loop(move || {
                queued.store(false, Ordering::Release);
                if !render.producer() {
                    frame_pacing::picture_set();
                }
            });
        });
    }
    /// GPU rendering: once, have the render thread's frame callback wake the
    /// UI thread for each new frame while PiP wants frames (coalesced; a
    /// wake-up that finds the host busy is dropped, the next frame comes).
    pub(crate) fn pip_gpu_hook(&mut self) {
        let (wanted, hooked) = &mut self.pip_gpu;
        wanted.store(self.pip.feeding(), Ordering::Release);
        let Some(gpu) = self.core.backend.render.gpu().filter(|_| !*hooked) else {
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
        self.core.backend.render.gpu().is_some() && !self.core.backend.render.producer()
    }
    /// The GPU renderer's newest frame to PiP, without a copy (each frame
    /// once). The renderer does not draw into a surface the system still
    /// shows, so the PiP window never sees a frame being redrawn.
    pub(crate) fn pip_gpu_frame(&mut self) {
        if !self.pip.feeding() || !self.pip_gpu_frames() {
            return;
        }
        let Some(gpu) = self.core.backend.render.gpu() else {
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
        let picture = self.core.backend.target.picture();
        let size = match picture {
            Some(picture) if self.pip.feeding() && !self.core.backend.render.producer() => {
                render_target::fit(area, picture)
            }
            _ => area,
        };
        if self.render_sent != Some(size) {
            self.render_sent = Some(size);
            self.core.backend.target.set_area(size);
            if let Some(gpu) = self.core.backend.render.gpu() {
                // `VESTRA_RENDER_TARGET` (diagnostics) applies to GPU frames too.
                let size = render_target::forced().unwrap_or(size);
                // A single step (fullscreen, maximize, a toggled column, PiP's
                // shape) gets new textures at once; a drag's sizes only once
                // one held.
                if self.render_continuous {
                    gpu.set_size(size);
                } else {
                    gpu.set_size_now(size);
                }
            }
        }
    }
    /// The video area as the layout has it now → the render size (C-SIZE)
    /// and the picture's shape for the video area's `video-clip-*` (shown
    /// alone while a frame of another shape is up, `picture_clip`). Each
    /// tick, and right after the window was resized (`RESIZE_SINK`), so the
    /// renderer starts on the new size before the next tick. Returns the
    /// area in logical pixels.
    pub(crate) fn follow_video_area(
        &mut self,
        shell: &slint_interpreter::ComponentInstance,
        fullscreen: bool,
    ) -> (f64, f64) {
        view::settle_layout(shell.window());
        let number = |name: &str| match shell.get_property(name) {
            Ok(SValue::Number(n)) => n,
            _ => 0.,
        };
        let video = (number("video-width"), number("video-height"));
        let scale = f64::from(shell.window().scale_factor());
        if let Some(applied) = self
            .sizing
            .observe(video, scale, fullscreen, Instant::now())
        {
            self.render_area = Some(applied.size);
            self.render_continuous = applied.continuous;
        }
        self.apply_render_size();
        // A generated stream's pictures are not mpv's frames.
        let aspect = (!self.core.backend.render.producer())
            .then(|| self.core.backend.target.layout())
            .flatten()
            .and_then(|layout| layout.letterbox_aspect())
            .unwrap_or(0.);
        if number("video-picture-aspect") != aspect {
            let _ = shell.set_property("video-picture-aspect", SValue::Number(aspect));
        }
        video
    }
    /// The main window was resized (its winit hook, after Slint's own
    /// handling): follow the new video area now instead of at the next tick.
    pub(crate) fn resized(&mut self) {
        if let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) {
            let fullscreen = shell.window().is_fullscreen();
            self.follow_video_area(&shell, fullscreen);
        }
    }
    /// A frame of `size` pixels may go to PiP: it has the shape the renderer
    /// was last asked for (a frame from before the switch to the picture's
    /// shape would size the PiP window wrongly). Generated streams' frames
    /// always may.
    pub(crate) fn pip_shape(&self, size: (u32, u32)) -> bool {
        self.core.backend.render.producer()
            || self
                .render_sent
                .is_none_or(|sent| render_target::same_shape(size, sent))
    }
    /// R-H11: a pending left press on the video becomes `Mouse:LeftClick`
    /// on release when nothing makes it wait (`video_click`), else once the
    /// double-click interval passes without a second press. It runs here,
    /// not through the event queue, so it costs no further loop turn.
    pub(crate) fn click_tick(&mut self, pointer: (f64, f64)) {
        // On the interface's copies of the tables (rule R8).
        self.sync_tables();
        let Some(click) = self.input.borrow().click else {
            return;
        };
        let input = bindings::Input {
            mods: click.mods,
            name: "Mouse:LeftClick".into(),
        };
        let immediate = video_click::immediate(&self.input.borrow().table, &*self.registry, &input);
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
                self.registry.resolve_execution(target.owner, &target.id),
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

#[cfg(test)]
mod tests {
    use super::count_winit_draw;
    use crate::playback::render_target::Presentation;
    use slint::winit_030::winit::event::WindowEvent;
    use std::time::Duration;

    /// 0.5.15: a software window counted its draws in a hook of its own,
    /// which the shell's event hook then replaced (`presented 0`).
    #[test]
    fn a_software_window_counts_its_redraws_in_the_shell_hook() {
        let presentation = Presentation::default();
        presentation.start_counting();
        presentation.picture_set(Duration::ZERO);
        assert!(count_winit_draw(
            true,
            &WindowEvent::RedrawRequested,
            &presentation
        ));
        assert!(!count_winit_draw(
            true,
            &WindowEvent::Focused(true),
            &presentation
        ));
        // The old picture again: a redraw, not a presented frame.
        assert!(count_winit_draw(
            true,
            &WindowEvent::RedrawRequested,
            &presentation
        ));
        assert_eq!(presentation.counts(), (2, 1));
        // Windows with a notifier or the pacing count: nothing here.
        assert!(!count_winit_draw(
            false,
            &WindowEvent::RedrawRequested,
            &presentation
        ));
        assert_eq!(presentation.counts(), (2, 1));
    }
}
