//! Binding a newly compiled shell's callbacks and its window's event hook.
use super::*;

impl Host {
    /// The main window's winit event hook, tagged with the shell generation
    /// that receives its events. The window has one hook slot, which outlives
    /// a shell (a new composition replaces the tree in the same window):
    /// it is set once the shell for `generation` is shown, never for a
    /// candidate that may still fail (`prepare_mount_tick`).
    pub(crate) fn bind_window_events(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) {
        use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
        // The window's close request quits the player, whatever tool windows
        // are still open (set again with each shell, like the hook below).
        let events = self.events.clone();
        view::on_main_close(window.window(), move || {
            let _ = events.send(Event::MainClosed);
        });
        let events = self.events.clone();
        #[cfg(feature = "producer-audio")]
        let routing = self.backend.input.clone();
        let input = self.input.clone();
        let weak = window.as_weak();
        let modifiers = std::cell::Cell::new(winit::keyboard::ModifiersState::empty());
        let menu_cursor = std::cell::Cell::new((f64::NAN, f64::NAN));
        // Software windows: this hook also counts the window's draws.
        let winit_draws = self.winit_draws.clone();
        let render = self.backend.render.clone();
        window.window().on_winit_window_event(move |slint_window, event| {
                video::count_winit_draw(winit_draws.get(), event, &render.presentation);
                if debug_menu()
                    && weak.upgrade().is_some_and(|shell| {
                        matches!(shell.get_property("menu-open"), Ok(SValue::Bool(true)))
                    })
                {
                    debug_menu_pointer(slint_window, &menu_cursor, event);
                }
                // Before Slint lays out and draws at the new size.
                if matches!(event, winit::event::WindowEvent::Resized(_)) {
                    platform::fullscreen_resized(slint_window);
                }
                track_pointer(&input, generation, slint_window, event);
                let dead = observe_key(&input, &modifiers, event);
                release_modifiers(&input, &events, generation);
                // Side buttons anywhere in the window (browser precedent).
                observe_side_button(&input, &modifiers, &events, generation, event, || {
                    weak.upgrade().is_some_and(|shell| {
                        matches!(shell.get_property("menu-open"), Ok(SValue::Bool(true)))
                    })
                });
                if let winit::event::WindowEvent::DroppedFile(path) = event {
                    let _ = events.send(Event::Dropped(generation, path.clone()));
                }
                if inject_option_key(&input, &modifiers, &events, generation, event) {
                    return EventResult::PreventDefault;
                }
                if let Some((dead, repeat)) = dead {
                    // Slint drops a dead key (it has no text), so its Option
                    // binding resolves here, only while the video area has
                    // focus: a focused text field composes with the dead key.
                    let focused = weak.upgrade().is_some_and(|shell| {
                        matches!(shell.get_property("canvas-focused"), Ok(SValue::Bool(true)))
                            && !matches!(shell.get_property("menu-open"), Ok(SValue::Bool(true)))
                    });
                    if focused && fire(&input, &events, generation, &dead, repeat, None) {
                        return EventResult::PreventDefault;
                    }
                }
                #[cfg(feature = "producer-audio")]
                match event {
                    winit::event::WindowEvent::Focused(false) | winit::event::WindowEvent::Occluded(true) | winit::event::WindowEvent::Destroyed => {
                        let _=events.send(Event::PhysicalInput(generation,json!({"kind":"revoke","reason":"focusLost"})));
                    }
                    winit::event::WindowEvent::KeyboardInput {event,is_synthetic:false,..} => {
                        let Some(shell) = weak.upgrade() else { return EventResult::Propagate; };
                        if !matches!(shell.get_property("canvas-focused"),Ok(slint_interpreter::Value::Bool(true)))
                            || matches!(shell.get_property("menu-open"),Ok(slint_interpreter::Value::Bool(true)))
                            || matches!(shell.get_property("input-blocked"),Ok(slint_interpreter::Value::Bool(true))) { return EventResult::Propagate; }
                        let routing = routing.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                        let Some(routing) = routing.filter(|routing| routing.captured) else { return EventResult::Propagate; };
                        if let winit::keyboard::PhysicalKey::Code(code) = event.physical_key {
                            let code = format!("{code:?}");
                            // Keys bound to the host's own commands (fullscreen, exit fullscreen) pass.
                            if host_key(&input, &code) {return EventResult::Propagate;}
                            if routing.keys.contains(&code) {
                                if !event.repeat {
                                    let _=events.send(Event::PhysicalInput(generation,json!({"kind":"key","scope":routing.scope,"code":code,"down":event.state==winit::event::ElementState::Pressed})));
                                }
                                return EventResult::PreventDefault;
                            }
                        }
                    }
                    _=>{}
                }
                EventResult::Propagate
            });
    }
    pub(crate) fn bind_shell(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        let events = self.events.clone();
        #[cfg(feature = "producer-audio")]
        let routing = self.backend.input.clone();
        window.set_callback("window-input", move |args| {
            if let Some(slint_interpreter::Value::String(kind)) = args.first() {
                #[cfg(feature = "producer-audio")]
                match kind.as_str() {
                    "capture" => {
                        if let Some(routing) = routing
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .clone()
                        {
                            let _ = events.send(Event::PhysicalInput(
                                generation,
                                json!({"kind":"capture","scope":routing.scope}),
                            ));
                        }
                    }
                    "canvasFocusLost" => {
                        let _ = events.send(Event::PhysicalInput(
                            generation,
                            json!({"kind":"revoke","reason":"focusLost"}),
                        ));
                    }
                    _ => {}
                }
                let _ = events.send(Event::WindowInput(generation, kind.to_string()));
            }
            slint_interpreter::Value::Void
        })?;
        self.bind_menu(window, generation, true)?;
        self.bind_keys(window, generation)?;
        let input = self.input.clone();
        let events = self.events.clone();
        window.set_callback("mouse-input", move |args| {
            let [SValue::String(name), mods @ ..] = args else {
                return SValue::Bool(false);
            };
            let flag = |i: usize| matches!(mods.get(i), Some(SValue::Bool(true)));
            let (ctrl, cmd) = slint_mods(flag(0), flag(3));
            let event = bindings::Input {
                mods: bindings::Mods {
                    ctrl,
                    alt: flag(1),
                    shift: flag(2),
                    cmd,
                },
                name: format!("Mouse:{name}"),
            };
            SValue::Bool(fire(&input, &events, generation, &event, false, None))
        })?;
        // G-05: dragging the video moves the window; the press is no click.
        let input = self.input.clone();
        let weak = window.as_weak();
        window.set_callback("drag-window", move |_| {
            input.borrow_mut().click = None;
            if let Some(window) = weak.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                window.window().with_winit_window(|window| {
                    // Systems without it (some Linux) refuse; the window stays.
                    let _ = window.drag_window();
                });
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("pip-return", move |_| {
            let _ = events.send(Event::PipReturn(generation));
            SValue::Void
        })?;
        let input = self.input.clone();
        window.set_callback("left-press", move |args| {
            let number = |i: usize| match args.get(i) {
                Some(SValue::Number(n)) if n.is_finite() => *n,
                _ => 0.,
            };
            let flag = |i: usize| matches!(args.get(i), Some(SValue::Bool(true)));
            let (ctrl, cmd) = slint_mods(flag(2), flag(5));
            let mut shared = input.borrow_mut();
            if !shared.table.has_left_click() {
                shared.click = None;
                return SValue::Void;
            }
            let (x, y) = (number(0), number(1));
            let now = Instant::now();
            // A second press within the double-click interval is a double
            // click; a first click still waiting for it is no click.
            let second = shared
                .click
                .take()
                .is_some_and(|first| first.doubled_by(x, y, now, double_click_interval()));
            input_trace::mark(if second {
                "video left press: a double click"
            } else {
                "video left press: a click pending"
            });
            if !second {
                let mods = bindings::Mods {
                    ctrl,
                    alt: flag(3),
                    shift: flag(4),
                    cmd,
                };
                shared.click = Some(video_click::Press::new(now, x, y, mods, generation));
            }
            SValue::Void
        })?;
        let input = self.input.clone();
        window.set_callback("left-release", move |args| {
            let number = |i: usize| match args.get(i) {
                Some(SValue::Number(n)) if n.is_finite() => *n,
                _ => 0.,
            };
            let mut shared = input.borrow_mut();
            if matches!(args.get(2), Some(SValue::Bool(true))) {
                shared.click = None;
            } else if let Some(click) = shared.click.as_mut().filter(|c| c.released.is_none()) {
                click.released = Some((number(0), number(1)));
                input_trace::mark("video left release");
            }
            SValue::Void
        })?;
        let input = self.input.clone();
        let events = self.events.clone();
        window.set_callback("wheel-input", move |args| {
            let number = |i: usize| match args.get(i) {
                Some(SValue::Number(n)) if n.is_finite() => *n,
                _ => 0.,
            };
            let flag = |i: usize| matches!(args.get(i), Some(SValue::Bool(true)));
            let (ctrl, cmd) = slint_mods(flag(2), flag(5));
            let mods = bindings::Mods {
                ctrl,
                alt: flag(3),
                shift: flag(4),
                cmd,
            };
            let (dx, dy) = (number(0), number(1));
            let (vertical, horizontal) = {
                let shared = input.borrow();
                (shared.table.has_wheel(true), shared.table.has_wheel(false))
            };
            if !vertical && !horizontal {
                return SValue::Bool(false);
            }
            let mut steps = Vec::new();
            {
                let mut shared = input.borrow_mut();
                shared.wheel.0 += dx;
                shared.wheel.1 += dy;
                while shared.wheel.1.abs() >= WHEEL_STEP {
                    let up = shared.wheel.1 > 0.;
                    shared.wheel.1 -= WHEEL_STEP.copysign(shared.wheel.1);
                    steps.push(if up { "WheelUp" } else { "WheelDown" });
                }
                while shared.wheel.0.abs() >= WHEEL_STEP {
                    let left = shared.wheel.0 > 0.;
                    shared.wheel.0 -= WHEEL_STEP.copysign(shared.wheel.0);
                    steps.push(if left { "WheelLeft" } else { "WheelRight" });
                }
            }
            for step in steps {
                let event = bindings::Input {
                    mods,
                    name: format!("Mouse:{step}"),
                };
                fire(&input, &events, generation, &event, false, None);
            }
            SValue::Bool(true)
        })?;
        let events = self.events.clone();
        window.set_callback("sidebar-resized", move |args| {
            if let Some(SValue::Number(width)) = args.first() {
                let _ = events.send(Event::SidebarResized(generation, *width));
            }
            SValue::Void
        })?;
        window.set_property("sidebar-width", self.policy.sidebar_user_width().into())?;
        window.set_property(
            "window-title",
            SValue::String(self.window_title.as_str().into()),
        )?;
        window.set_property(
            "info-panels",
            ui_values::from_json(&json!(self.info_panels.values().collect::<Vec<_>>()))?,
        )?;
        let roots = self.commands.snapshot().roots;
        let labels = self.labels();
        // Through `apply_menu_levels`, so `menu_shown` follows the window:
        // a new shell's `menu-levels` is not the model it remembers, so this
        // is a whole write, and later hovers write in place again. A
        // candidate shell that is then dropped leaves `menu_shown` naming
        // its model, which the kept shell does not hold: its next write is
        // a whole one too.
        let levels = menu_levels(
            window,
            &roots,
            &mut self.menu_path.clone(),
            &self.menu_scroll,
            &labels,
        )?
        .0;
        apply_menu_levels(window, &mut self.menu_shown.borrow_mut(), &levels)?;
        if let Some(image) = &self.last_image {
            window.set_property("video-frame", image.clone().into())?;
        }
        window.set_property("no-video", self.no_video.into())?;
        Ok(())
    }
}
