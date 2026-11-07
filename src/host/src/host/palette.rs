//! The command palette (`host/palette`, Ctrl+Shift+P): opened on the
//! main window's menu layer as a menu mode (`MenuMode::Palette`), so
//! bindings pause, the window counts it as an obstruction and it closes
//! like a menu. The listing, matching and rows are `ui::command_palette`.
use super::*;
use crate::ui::command_palette::{self, Hit, Item};

/// The open palette: what is typed, the cursor and the first row in view
/// (indexes into `hits`), and what the last projection listed.
#[derive(Default)]
pub(crate) struct PaletteState {
    pub(crate) query: String,
    pub(crate) cursor: usize,
    pub(crate) offset: usize,
    pub(crate) items: Vec<Item>,
    pub(crate) hits: Vec<Hit>,
    /// Rows in view at the last projection.
    pub(crate) visible: usize,
    /// The command the cursor goes to at the next projection (its global
    /// id): after Shift+Enter it moves up the recent list, and the cursor
    /// goes with it.
    pub(crate) follow: Option<String>,
}

/// What the palette view asks for.
pub(crate) enum PaletteAction {
    /// The text in the field changed.
    Query(String),
    /// Move the cursor by this many rows (the host clamps it).
    Move(i64),
    /// The wheel: move the rows in view by this many.
    Scroll(i64),
    /// Run the row at this slot, or the cursor's (`None`); `true` keeps the
    /// palette open.
    Run(Option<usize>, bool),
}

impl Host {
    /// Open the palette in the main window, closing any open menu.
    pub(crate) fn open_palette(&mut self) {
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        if self.menu_open {
            self.close_menu();
        }
        #[cfg(feature = "producer-audio")]
        self.physical_revoke(producer_input::RevokeReason::Menu);
        self.menu_open = true;
        self.menu_generation = self.shell_generation;
        self.menu_mode = MenuMode::Palette;
        self.palette = PaletteState::default();
        let result = (|| -> Result<()> {
            shell.set_property("palette-query", SValue::String("".into()))?;
            // The footer's keys as the menus write them on this system.
            shell.set_property(
                "palette-keys",
                ui_values::from_json(&json!(command_palette::footer_keys()))?,
            )?;
            shell.set_property("palette-open", true.into())?;
            shell.set_property("menu-open", true.into())?;
            Ok(())
        })();
        self.report_menu(result);
        let result = self.refresh_menu();
        self.report_menu(result);
    }
    /// The shown name of each plugin with commands, by manifest id.
    fn plugin_names(&self) -> BTreeMap<String, String> {
        let mut names = BTreeMap::new();
        for plugin in self.plugins.values() {
            let catalog = plugin.origin().catalog;
            names.insert(
                plugin.manifest.id.clone(),
                package_manager::display_name(&plugin.manifest, catalog.as_deref()),
            );
        }
        names
    }
    /// Project the open palette into `window` (`refresh_menu`): the
    /// commands as they are now, matched against the query.
    pub(crate) fn refresh_palette(
        &mut self,
        window: &slint_interpreter::ComponentInstance,
    ) -> Result<()> {
        let names = self.plugin_names();
        let name = |id: &str| {
            names
                .get(id)
                .cloned()
                .or_else(|| self.on_demand.name_of(id).map(str::to_owned))
                .unwrap_or_else(|| id.to_owned())
        };
        let labels = self.labels();
        let entries = self.commands.shown_entries();
        let items = command_palette::items(&entries, &name, &labels);
        let hits = command_palette::search(
            &items,
            &self.palette.query,
            &self.ui_state.state.palette_recent,
        );
        let (_, size) = super::menus::menu_frame(window);
        let visible = command_palette::visible(size.y);
        let palette = &mut self.palette;
        if let Some(global) = palette.follow.take()
            && let Some(at) = command_palette::position(&items, &hits, &global)
        {
            palette.cursor = at;
        }
        palette.cursor = palette.cursor.min(hits.len().saturating_sub(1));
        palette.offset =
            command_palette::reveal(palette.offset, palette.cursor, visible, hits.len());
        let rows = command_palette::rows(&items, &hits, palette.offset, visible);
        let current = if hits.is_empty() {
            -1
        } else {
            (palette.cursor - palette.offset) as i64
        };
        let shown = hits.len().saturating_sub(palette.offset).min(visible);
        let count = i18n::tr_args("Results: {}", &json!([hits.len()]));
        palette.items = items;
        palette.hits = hits;
        palette.visible = visible;
        let next = ui_values::from_json(&rows)?;
        let live = window.get_property("palette-rows")?;
        if !ui_values::reconcile_nested(&live, &next)? {
            window.set_property("palette-rows", next)?;
        }
        window.set_property("palette-current", SValue::Number(current as f64))?;
        window.set_property("palette-shown", SValue::Number(shown as f64))?;
        window.set_property("palette-count", SValue::String(count.as_str().into()))?;
        Ok(())
    }
    pub(crate) fn palette_action(&mut self, action: PaletteAction) {
        if self.menu_mode != MenuMode::Palette {
            return;
        }
        match action {
            PaletteAction::Query(query) => {
                if query == self.palette.query {
                    return;
                }
                self.palette.query = query;
                self.palette.cursor = 0;
                self.palette.offset = 0;
            }
            PaletteAction::Move(by) => {
                let count = self.palette.hits.len();
                if count == 0 {
                    return;
                }
                let last = count as i64 - 1;
                let at = self.palette.cursor as i64;
                // Up from the first row goes to the last and back, as
                // menus do; a page stops at either end.
                self.palette.cursor = match by {
                    -1 if at == 0 => last,
                    1 if at == last => 0,
                    _ => (at + by).clamp(0, last),
                } as usize;
            }
            PaletteAction::Scroll(by) => {
                let count = self.palette.hits.len();
                let visible = self.palette.visible.max(1);
                let most = count.saturating_sub(visible) as i64;
                let offset = (self.palette.offset as i64 + by).clamp(0, most) as usize;
                self.palette.offset = offset;
                // The cursor stays in view.
                self.palette.cursor = self
                    .palette
                    .cursor
                    .clamp(offset, (offset + visible - 1).min(count.saturating_sub(1)));
            }
            PaletteAction::Run(slot, keep) => {
                let index = match slot {
                    Some(slot) => self.palette.offset + slot,
                    None => self.palette.cursor,
                };
                let Some(item) = self
                    .palette
                    .hits
                    .get(index)
                    .and_then(|hit| self.palette.items.get(hit.index))
                    .cloned()
                else {
                    return;
                };
                if !item.enabled || item.busy {
                    return;
                }
                // The palette's own command run from the palette just
                // closes it (running it would open the palette again).
                if item.owner == bindings::HOST && item.id == PALETTE {
                    self.close_menu();
                    return;
                }
                command_palette::remember(&mut self.ui_state.state.palette_recent, &item.global);
                let now = self.now_ms();
                self.ui_state.touch(now);
                // Kept open: the cursor stays on this command wherever it
                // now sorts (the recent list reorders an empty query).
                self.palette.cursor = index;
                self.palette.follow = Some(item.global.clone());
                if !keep {
                    self.close_menu();
                }
                if let Err(error) = self.execute_command(item.owner, &item.id) {
                    diag!("palette execute: {:?}", error);
                }
                if !keep {
                    return;
                }
            }
        }
        let result = self.refresh_menu();
        self.report_menu(result);
    }
    /// The palette view's callbacks in the main window.
    pub(crate) fn bind_palette(
        &self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        let send = move |events: &mpsc::Sender<Event>, action: PaletteAction| {
            let _ = events.send(Event::MenuAction(generation, MenuAction::Palette(action)));
        };
        let events = self.events.clone();
        window.set_callback("palette-edited", move |args| {
            if let Some(SValue::String(text)) = args.first() {
                send(&events, PaletteAction::Query(text.to_string()));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("palette-moved", move |args| {
            if let Some(SValue::Number(by)) = args.first()
                && by.is_finite()
            {
                send(&events, PaletteAction::Move(*by as i64));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("palette-scrolled", move |args| {
            if let Some(SValue::Number(by)) = args.first()
                && by.is_finite()
            {
                send(&events, PaletteAction::Scroll(*by as i64));
            }
            SValue::Void
        })?;
        let events = self.events.clone();
        window.set_callback("palette-run", move |args| {
            let slot = match args.first() {
                Some(SValue::Number(slot)) if *slot >= 0. => Some(*slot as usize),
                _ => None,
            };
            let keep = matches!(args.get(1), Some(SValue::Bool(true)));
            send(&events, PaletteAction::Run(slot, keep));
            SValue::Void
        })?;
        Ok(())
    }
}

/// The palette's own command id (`host/palette`).
pub(crate) const PALETTE: &str = "palette";
