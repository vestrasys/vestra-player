//! The context menu: opening, model, paths, levels and pointer aim.
use super::*;

/// Projects the open chain at the origin the view recorded for this opening.
pub(crate) fn menu_levels(
    window: &slint_interpreter::ComponentInstance,
    roots: &[commands::MenuNode],
    path: &mut Vec<String>,
    scroll: &[f64],
    labels: &dyn Fn(u64, &str) -> Option<String>,
) -> Result<(slint_interpreter::Value, Vec<menu::Rect>)> {
    *path = menu::valid_path(roots, path);
    let (origin, size) = menu_frame(window);
    let value = menu::levels(roots, path, origin, size, scroll, labels);
    if debug_menu() {
        let panels: Vec<String> = value
            .as_array()
            .into_iter()
            .flatten()
            .map(|level| {
                let number = |key: &str| level[key].as_f64().unwrap_or(f64::NAN);
                format!(
                    "x {:.0}..{:.0} y {:.0}..{:.0}",
                    number("x"),
                    number("x") + number("width"),
                    number("y"),
                    number("y") + number("height")
                )
            })
            .collect();
        diag!(
            "menu: open={path:?} window={:.0}x{:.0} panels=[{}]",
            size.x,
            size.y,
            panels.join("; ")
        );
    }
    Ok((ui_values::from_json(&value)?, menu::panels(&value)))
}

/// Where the menu opens and the window it must fit in, in logical px.
pub(crate) fn menu_frame(
    window: &slint_interpreter::ComponentInstance,
) -> (menu::Point, menu::Point) {
    let number = |name: &str| match window.get_property(name) {
        Ok(slint_interpreter::Value::Number(value)) if value.is_finite() => value,
        _ => 0.,
    };
    (
        menu::Point {
            x: number("menu-origin-x"),
            y: number("menu-origin-y"),
        },
        menu::Point {
            x: number("logical-width"),
            y: number("logical-height"),
        },
    )
}

/// `VESTRA_DEBUG_KEYS=1`: log every key event and how it resolved to stderr.
/// `VESTRA_DEBUG_MENU=1`: the context menu's hovers, invocations and panel
/// geometry on stderr.
pub(crate) fn debug_menu() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VESTRA_DEBUG_MENU").is_some_and(|v| v == "1"))
}

/// `VESTRA_DEBUG_MENU=1`: the pointer as Slint receives it (logical px in
/// the window's content), so a log shows which row a move or press hits.
/// Record where the pointer is in this window (`InputShared::pointer`).
pub(crate) fn track_pointer(
    input: &Rc<RefCell<InputShared>>,
    generation: u64,
    window: &slint::Window,
    event: &slint::winit_030::winit::event::WindowEvent,
) {
    use slint::winit_030::winit::event::WindowEvent;
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            let scale = f64::from(window.scale_factor());
            let point = menu::Point {
                x: position.x / scale,
                y: position.y / scale,
            };
            input.borrow_mut().pointer = Some((generation, point));
        }
        WindowEvent::CursorLeft { .. } => {
            let mut shared = input.borrow_mut();
            if shared.pointer.is_some_and(|(owner, _)| owner == generation) {
                shared.pointer = None;
            }
        }
        _ => {}
    }
}

pub(crate) fn debug_menu_pointer(
    window: &slint::Window,
    cursor: &std::cell::Cell<(f64, f64)>,
    event: &slint::winit_030::winit::event::WindowEvent,
) {
    use slint::winit_030::winit::event::WindowEvent;
    let scale = f64::from(window.scale_factor());
    match event {
        WindowEvent::CursorMoved { position, .. } => {
            let (x, y) = (position.x / scale, position.y / scale);
            cursor.set((x, y));
            diag!("menu: pointer moved x={x:.1} y={y:.1}");
        }
        WindowEvent::MouseInput { state, button, .. } => {
            let (x, y) = cursor.get();
            diag!("menu: pointer {state:?} {button:?} at x={x:.1} y={y:.1}");
        }
        _ => {}
    }
}

/// Registry revisions include changes outside the open levels and identical
/// state publications. Reconcile in place at every depth so the rows keep
/// their live elements: a state published while a row is pressed must not
/// cancel the click, nor a switched submenu the hover of its parent's rows.
pub(crate) fn set_menu_model(
    window: &slint_interpreter::ComponentInstance,
    next: slint_interpreter::Value,
) -> Result<()> {
    let current = window.get_property("menu-levels")?;
    if !ui_values::reconcile_nested(&current, &next)? {
        window.set_property("menu-levels", next)?;
    }
    // A level whose own fields changed (its open row, its size) leaves its
    // rows' repeater with a dirty model binding until the next frame, and
    // Slint 1.18 drops a row change that arrives meanwhile when the binding
    // then yields the same model (`RepeaterTracker::row_changed` marks the
    // row dirty; `model()` sees an unchanged model and never updates it).
    // The host writes the menu several times between two frames when the
    // pointer crosses rows fast (hover events are handled in 16 ms
    // batches): a group row opened and closed in one batch kept `open`
    // and stayed lit (user report, 0.5.9). Settling the repeaters now
    // makes the next in-place write apply at once.
    view::settle_layout(window.window());
    Ok(())
}

impl Host {
    pub(crate) fn report_menu(&mut self, result: Result<()>) {
        if let Err(error) = result {
            self.broadcast(
                "host.error",
                json!({"code":"menu-bridge","message":error.to_string()}),
            );
        }
    }
    /// Menu callbacks shared by the shell and the floating window.
    pub(crate) fn bind_menu(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
        reset: bool,
    ) -> Result<()> {
        let events = self.events.clone();
        window.set_callback("menu-hover", move |args| {
            if let [
                SValue::Number(level),
                SValue::String(token),
                SValue::Bool(now),
            ] = args
                && *level >= 0.
            {
                let _ = events.send(Event::MenuAction(
                    generation,
                    MenuAction::Hover(*level as usize, token.to_string(), *now),
                ));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("menu-scroll", move |args| {
            if let [SValue::Number(level), SValue::Number(offset)] = args
                && *level >= 0.
            {
                let _ = events.send(Event::MenuAction(
                    generation,
                    MenuAction::Scroll(*level as usize, *offset),
                ));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("menu-invoke", move |args| {
            if let Some(SValue::String(token)) = args.first() {
                let _ = events.send(Event::MenuAction(
                    generation,
                    MenuAction::Invoke(token.to_string()),
                ));
            }
            SValue::Void
        })?;
        let mut callbacks = vec!["menu-back", "menu-dismiss"];
        if reset {
            callbacks.push("menu-reset");
        }
        for callback in callbacks {
            let events = self.events.clone();
            window.set_callback(callback, move |_| {
                let action = match callback {
                    "menu-reset" => MenuAction::Reset,
                    "menu-back" => MenuAction::Back,
                    _ => MenuAction::Dismiss,
                };
                let _ = events.send(Event::MenuAction(generation, action));
                SValue::Void
            })?;
        }
        Ok(())
    }
    pub(crate) fn labels(&self) -> impl Fn(u64, &str) -> Option<String> + use<> {
        let input = self.input.clone();
        move |owner, id| input.borrow().table.label(owner, id)
    }
    /// Show the menu in the window of `generation` (shell or floating).
    pub(crate) fn open_menu(&mut self, generation: u64, mode: MenuMode) {
        if self.menu_open && self.menu_generation != generation {
            self.close_menu();
        }
        self.menu_open = true;
        self.menu_generation = generation;
        self.menu_mode = mode;
        self.menu_path.clear();
        self.menu_scroll.clear();
        self.menu_aim.clear();
        if let Some(window) = self.window_for(generation) {
            let _ = window.set_property("menu-open", true.into());
        }
        let result = self.refresh_menu();
        self.report_menu(result);
    }
    /// Open this chain of submenus now. Levels that close or change forget
    /// their scroll offset; a waiting delayed hover is dropped.
    pub(crate) fn set_menu_path(&mut self, path: Vec<String>) {
        self.menu_aim.clear();
        if path == self.menu_path {
            return;
        }
        let common = path
            .iter()
            .zip(&self.menu_path)
            .take_while(|(a, b)| a == b)
            .count();
        self.menu_scroll.truncate(common + 1);
        self.menu_path = path;
        let result = self.refresh_menu();
        self.report_menu(result);
    }
    /// G-15: apply a submenu switch held while the pointer headed for the
    /// open submenu, once it turned away or stopped.
    pub(crate) fn menu_tick(&mut self) {
        if !self.menu_open {
            return;
        }
        let now = self.now_ms();
        let pointer = self.menu_pointer();
        if let Some(path) = self.menu_aim.tick(pointer, now) {
            self.set_menu_path(path);
        }
    }
    /// The pointer in the window the menu is open in (logical px).
    pub(crate) fn menu_pointer(&self) -> Option<menu::Point> {
        self.input
            .borrow()
            .pointer
            .filter(|(generation, _)| *generation == self.menu_generation)
            .map(|(_, point)| point)
    }
    pub(crate) fn refresh_menu(&mut self) -> Result<()> {
        let labels = self.labels();
        let Some(window) = self
            .window_for(self.menu_generation)
            .or(self.shell.as_ref())
            .map(ComponentHandle::clone_strong)
        else {
            return Ok(());
        };
        let window = &window;
        let model = match &self.menu_mode {
            MenuMode::Main => {
                let roots = self.commands.snapshot().roots;
                let (model, panels) = menu_levels(
                    window,
                    &roots,
                    &mut self.menu_path,
                    &self.menu_scroll,
                    &labels,
                )?;
                self.menu_panels = panels;
                model
            }
            MenuMode::Panel(owner, items) => {
                // A command removed while its panel menu is open drops out.
                let entries: Vec<_> = items
                    .iter()
                    .filter_map(|id| match id {
                        None => Some(None),
                        Some(id) => self.commands.get(*owner, id).map(Some),
                    })
                    .collect();
                let (origin, size) = menu_frame(window);
                let value = menu::context_level(
                    &entries,
                    origin,
                    size,
                    self.menu_scroll.first().copied().unwrap_or(0.),
                    &labels,
                );
                self.menu_panels = menu::panels(&value);
                ui_values::from_json(&value)?
            }
        };
        set_menu_model(window, model)
    }
    pub(crate) fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_path.clear();
        self.menu_scroll.clear();
        self.menu_aim.clear();
        self.menu_mode = MenuMode::Main;
        for window in [
            self.shell.as_ref(),
            self.floating.as_ref().map(|f| &f.instance),
        ]
        .into_iter()
        .flatten()
        {
            let _ = window.set_property("menu-open", false.into());
        }
        if let Some(shell) = &self.shell {
            let result = shell
                .invoke("close-menu", &[])
                .map(|_| ())
                .map_err(anyhow::Error::from);
            self.report_menu(result);
        }
    }
}
