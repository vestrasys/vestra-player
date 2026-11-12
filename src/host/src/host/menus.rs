//! The context menu: opening, model, paths, levels and pointer aim.
use super::*;
use slint::Model as _;

/// Projects the open chain at the origin the view recorded for this opening
/// (`menu::levels`, as JSON: `apply_menu_levels` writes only what changed).
pub(crate) fn menu_levels(
    window: &slint_interpreter::ComponentInstance,
    roots: &[commands::MenuNode],
    path: &mut Vec<String>,
    scroll: &[f64],
    labels: &dyn Fn(u64, &str) -> Option<String>,
) -> Result<(Value, Vec<menu::Rect>)> {
    *path = menu::valid_path(roots, path);
    let (origin, size) = menu_frame(window);
    let area = menu_area(window, origin, size);
    let value = menu::within(area, origin, |origin, size| {
        menu::levels(roots, path, origin, size, scroll, labels)
    });
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
            "menu: open={path:?} window={:.0}x{:.0} area={:.0},{:.0}..{:.0},{:.0} panels=[{}]",
            size.x,
            size.y,
            area.left,
            area.top,
            area.right,
            area.bottom,
            panels.join("; ")
        );
    }
    let panels = menu::panels(&value);
    Ok((value, panels))
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

/// The part of `window` on the screen holding the menu's origin
/// (`menu::visible_area`): menus flip and shift at the screen's edge, also
/// when the window reaches past it. The whole window when the system does
/// not say where the window or its screens are (headless, Wayland).
pub(crate) fn menu_area(
    window: &slint_interpreter::ComponentInstance,
    origin: menu::Point,
    size: menu::Point,
) -> menu::Area {
    use slint::winit_030::WinitWindowAccessor;
    let scale = f64::from(window.window().scale_factor());
    // The displays' work areas (without the taskbar), as the system's menus
    // keep to them: Windows lists them (`platform::work_areas`, the same
    // desktop pixels as winit's positions); elsewhere winit gives only
    // whole displays.
    let work: Vec<_> = crate::platform::work_areas()
        .into_iter()
        .map(|(area, _)| {
            (
                f64::from(area.x),
                f64::from(area.y),
                f64::from(area.width),
                f64::from(area.height),
            )
        })
        .collect();
    let screens = window.window().with_winit_window(|winit| {
        let client = winit.inner_position().ok()?;
        let monitors: Vec<_> = if work.is_empty() {
            winit
                .available_monitors()
                .map(|monitor| {
                    let (at, extent) = (monitor.position(), monitor.size());
                    (
                        f64::from(at.x),
                        f64::from(at.y),
                        f64::from(extent.width),
                        f64::from(extent.height),
                    )
                })
                .collect()
        } else {
            work.clone()
        };
        Some(((f64::from(client.x), f64::from(client.y)), monitors))
    });
    match screens.flatten() {
        Some((client, monitors)) => menu::visible_area(size, origin, client, scale, &monitors),
        None => menu::Area {
            left: 0.,
            top: 0.,
            right: size.x,
            bottom: size.y,
        },
    }
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

/// The levels last written into a window's `menu-levels` (the model and
/// the JSON it holds), so the next write changes only what differs.
#[derive(Default)]
pub(crate) struct ShownMenu {
    model: Option<slint::ModelRc<SValue>>,
    levels: Vec<Value>,
}

/// What `apply_menu_levels` wrote (the tests count it).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MenuWrite {
    /// The whole value went through `set_menu_model` (first write to a
    /// window, or its model was replaced).
    pub full: bool,
    /// Rows converted and written (changed, added) or removed.
    pub rows: usize,
    /// Levels whose own fields were written, added or removed.
    pub levels: usize,
    /// `view::settle_layout` ran.
    pub settled: bool,
}

/// Write the projected `levels` (`menu::levels`) into `window`'s
/// `menu-levels`, changing only what differs from the last write
/// (`shown`). A hover that switches a submenu writes the hovered level's
/// two rows whose `open` changed and that level's `open`, and the levels
/// below it; the other levels are not converted or compared again (host
/// 0.5.26: before, every hover converted and reconciled the whole menu and
/// settled the window, about 3 ms). A republished state writes its row.
///
/// Rows are written in place (their elements stay, keeping a press in
/// progress and the hover). Only a level's own fields (its open row, its
/// geometry) and added or removed levels leave a repeater with a dirty
/// model binding (`set_menu_model`): only then is the window settled.
pub(crate) fn apply_menu_levels(
    window: &slint_interpreter::ComponentInstance,
    shown: &mut ShownMenu,
    levels: &Value,
) -> Result<MenuWrite> {
    let next = levels.as_array().map(Vec::as_slice).unwrap_or_default();
    let live = window.get_property("menu-levels")?;
    let model = match (&live, &shown.model) {
        (SValue::Model(live), Some(model)) if live == model => Some(model.clone()),
        _ => None,
    };
    let write = match model
        .as_ref()
        .and_then(|model| model.as_any().downcast_ref::<crate::ui::ui_rows::Rows>())
    {
        // A write that fails midway leaves some rows written and `shown`
        // stale: forget what is shown, so the next write is a whole one.
        Some(model) => match write_levels(model, &shown.levels, next) {
            Ok(write) => write,
            Err(error) => {
                *shown = ShownMenu::default();
                return Err(error);
            }
        },
        None => None,
    };
    let write = match write {
        Some(write) => {
            if write.settled {
                view::settle_layout(window.window());
            }
            write
        }
        None => {
            // Same if the whole write fails.
            *shown = ShownMenu::default();
            set_menu_model(window, ui_values::from_json(levels)?)?;
            shown.model = match window.get_property("menu-levels")? {
                SValue::Model(model) => Some(model),
                _ => None,
            };
            MenuWrite {
                full: true,
                rows: 0,
                levels: next.len(),
                settled: true,
            }
        }
    };
    shown.levels = next.to_vec();
    Ok(write)
}

/// `apply_menu_levels` in place; `None` when the live model does not hold
/// what `shown` says (the caller writes the whole value).
fn write_levels(
    model: &crate::ui::ui_rows::Rows,
    shown: &[Value],
    next: &[Value],
) -> Result<Option<MenuWrite>> {
    if model.row_count() != shown.len() {
        return Ok(None);
    }
    let mut write = MenuWrite::default();
    let shared = shown.len().min(next.len());
    for (index, (before, after)) in shown.iter().zip(next).enumerate() {
        if before == after {
            continue;
        }
        let Some(SValue::Struct(level)) = model.row_data(index) else {
            return Ok(None);
        };
        let rows = match level.get_field("rows") {
            Some(SValue::Model(rows)) => rows.clone(),
            _ => return Ok(None),
        };
        let Some(rows_model) = rows.as_any().downcast_ref::<crate::ui::ui_rows::Rows>() else {
            return Ok(None);
        };
        let empty = Vec::new();
        let old_rows = before["rows"].as_array().unwrap_or(&empty);
        let new_rows = after["rows"].as_array().unwrap_or(&empty);
        if rows_model.row_count() != old_rows.len() {
            return Ok(None);
        }
        // The rows first: a row written after its level's own fields
        // would wait for the settle (`set_menu_model`).
        for (row, (old, new)) in old_rows.iter().zip(new_rows).enumerate() {
            if old != new {
                rows_model.set_row_data(row, ui_values::from_json(new)?);
                write.rows += 1;
            }
        }
        while rows_model.row_count() > new_rows.len() {
            rows_model.remove(rows_model.row_count() - 1);
            write.rows += 1;
        }
        for new in new_rows.iter().skip(old_rows.len()) {
            rows_model.push(ui_values::from_json(new)?);
            write.rows += 1;
        }
        // The level's own fields (open row, geometry, scrolling).
        let changed: Vec<_> = after
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(name, value)| {
                name.as_str() != "rows" && before.get(name.as_str()) != Some(*value)
            })
            .collect();
        if !changed.is_empty() {
            let mut level = level;
            for (name, value) in changed {
                level.set_field(name.clone(), ui_values::from_json(value)?);
            }
            model.set_row_data(index, SValue::Struct(level));
            write.levels += 1;
            write.settled = true;
        }
    }
    while model.row_count() > next.len() {
        model.remove(model.row_count() - 1);
        write.levels += 1;
        write.settled = true;
    }
    for level in next.iter().skip(shared) {
        model.push(ui_values::from_json(level)?);
        write.levels += 1;
        write.settled = true;
    }
    Ok(Some(write))
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
        self.review_hidden();
        self.menu_open = true;
        self.menu_generation = generation;
        self.menu_mode = mode;
        self.menu_path.clear();
        self.menu_scroll.clear();
        self.menu_aim.clear();
        if let Some(window) = self.window_for(generation) {
            let _ = window.set_property("menu-open", true.into());
            // A menu replaces an open palette (the floating window has none).
            let _ = window.set_property("palette-open", false.into());
            // And the approval sheet, which shows again at the next tick.
            let _ = window.set_property("review-open", false.into());
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
                let snapshot = self.commands.snapshot();
                let (model, panels) = menu_levels(
                    window,
                    &snapshot.roots,
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
                let scroll = self.menu_scroll.first().copied().unwrap_or(0.);
                let value = menu::within(menu_area(window, origin, size), origin, |o, s| {
                    menu::context_level(&entries, o, s, scroll, &labels)
                });
                self.menu_panels = menu::panels(&value);
                value
            }
            MenuMode::Palette => return self.refresh_palette(window),
            // Written whole when it shows (`present_review`).
            MenuMode::Review => return Ok(()),
            MenuMode::Bar(items) => {
                // A button whose command went while `»` is open drops out.
                let entries: Vec<_> = items
                    .iter()
                    .filter_map(|(owner, id)| self.commands.get(*owner, id))
                    .collect();
                let (origin, size) = menu_frame(window);
                let scroll = self.menu_scroll.first().copied().unwrap_or(0.);
                let value = menu::within(menu_area(window, origin, size), origin, |o, s| {
                    menu::bar_level(&entries, o, s, scroll, &labels)
                });
                self.menu_panels = menu::panels(&value);
                value
            }
            MenuMode::Tabs(column) => {
                let column = *column;
                // Every tab, in that window's tab order (sidebar-tabs.md).
                let owners = match &self.floating {
                    Some(floating)
                        if floating.generation == self.menu_generation
                            && column == sidebar_tabs::Column::Sidebar =>
                    {
                        &floating.owners
                    }
                    _ => &self.columns[column.index()].tabs,
                };
                let tabs = owners
                    .iter()
                    .filter_map(|owner| self.mounts.get(owner).map(|m| (*owner, m.title.clone())))
                    .collect::<Vec<_>>();
                let (origin, size) = menu_frame(window);
                let scroll = self.menu_scroll.first().copied().unwrap_or(0.);
                let active = self.policy.active(super::mounts::policy_column(column));
                let value = menu::within(menu_area(window, origin, size), origin, |o, s| {
                    menu::tabs_level(&tabs, active, o, s, scroll)
                });
                self.menu_panels = menu::panels(&value);
                value
            }
        };
        let write = apply_menu_levels(window, &mut self.menu_shown.borrow_mut(), &model)?;
        if debug_menu() {
            diag!("menu: wrote {write:?}");
        }
        Ok(())
    }
    pub(crate) fn close_menu(&mut self) {
        self.menu_open = false;
        self.menu_path.clear();
        self.menu_scroll.clear();
        self.menu_aim.clear();
        self.menu_mode = MenuMode::Main;
        self.palette = palette::PaletteState::default();
        // An approval sheet still waiting shows again at the next tick.
        self.review_hidden();
        for window in [
            self.shell.as_ref(),
            self.floating.as_ref().map(|f| &f.instance),
        ]
        .into_iter()
        .flatten()
        {
            let _ = window.set_property("menu-open", false.into());
            // The floating window has no palette: its refusal is nothing.
            let _ = window.set_property("palette-open", false.into());
            let _ = window.set_property("review-open", false.into());
        }
        if let Some(shell) = &self.shell {
            let result = shell
                .invoke("close-menu", &[])
                .map(|_| ())
                .map_err(anyhow::Error::from);
            self.report_menu(result);
        }
        // The status strip comes back (it yields to a menu).
        self.refresh_strip();
    }
}
