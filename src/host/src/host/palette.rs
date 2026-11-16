//! The command palette (`host/palette`, Ctrl+Shift+P): opened on the
//! main window's menu layer as a menu mode (`MenuMode::Palette`), so
//! bindings pause, the window counts it as an obstruction and it closes
//! like a menu. The listing, matching and rows are `ui::command_palette`.
//!
//! A built-in plugin may hold the palette's first row (`palette.lead`,
//! capabilities/palette_lead.rs): drawn above the commands in view, it
//! receives what is typed on Enter. The cursor indexes the rows with the
//! lead first (0) while there is one; without one nothing changes.
use super::*;
use crate::ui::command_palette::{self, Hit, Item};

/// The open palette: what is typed, the cursor and the first row in view
/// (indexes into `hits`), and what the last projection listed.
#[derive(Default)]
pub(crate) struct PaletteState {
    pub(crate) query: String,
    /// The cursor among the rows: the lead (0, while `lead`), then `hits`.
    pub(crate) cursor: usize,
    /// The first command in view (an index into `hits`).
    pub(crate) offset: usize,
    pub(crate) items: Vec<Item>,
    pub(crate) hits: Vec<Hit>,
    /// Command rows in view at the last projection.
    pub(crate) visible: usize,
    /// The command the cursor goes to at the next projection (its global
    /// id): after Shift+Enter it moves up the recent list, and the cursor
    /// goes with it.
    pub(crate) follow: Option<String>,
    /// The last projection showed a lead row first.
    pub(crate) lead: bool,
    /// The last projection's lead row showed its holder's status.
    pub(crate) status: bool,
    /// Shift+Enter sent a request from this palette and cleared the field;
    /// until its status has come and gone (or something is typed, or the
    /// request is stopped), Enter on the lead row with nothing typed stops
    /// it, also before the holder's status arrives (a parked holder takes
    /// a moment to wake).
    pub(crate) sent: bool,
    /// The next projection places the cursor by the lead's rule
    /// (`command_palette::aim`): after opening, typing, the lead coming or
    /// going, or its status coming or going while nothing is typed.
    pub(crate) aim: bool,
}

impl PaletteState {
    /// Rows the cursor moves over: the lead's, then the commands.
    fn rows(&self) -> usize {
        usize::from(self.lead) + self.hits.len()
    }
    /// The command under the cursor (an index into `hits`), `None` on the lead.
    fn command_at_cursor(&self) -> Option<usize> {
        self.cursor.checked_sub(usize::from(self.lead))
    }
    /// Shift+Enter sent the query: the field is cleared and the cursor
    /// stays on the lead row (not placed by `aim`, which with nothing typed
    /// and no status yet would pick the most recent command, so a second
    /// Enter would run it); the status, once it comes, re-aims onto itself.
    fn kept_sent(&mut self) {
        self.query.clear();
        self.cursor = 0;
        self.offset = 0;
        self.aim = false;
        self.follow = None;
        self.sent = true;
    }
    /// The lead row and its status as the next projection shows them:
    /// whether the cursor is placed again (`aim`). The lead coming or going
    /// re-aims; so does its status coming or going while nothing is typed.
    /// The status going ends `sent` (that request is over).
    fn track_lead(&mut self, lead: bool, status: bool) {
        if self.lead != lead {
            self.lead = lead;
            self.aim = true;
        }
        if self.status != status {
            if !status {
                self.sent = false;
            }
            self.status = status;
            self.aim |= self.query.trim().is_empty();
        }
    }
}

/// What the palette view asks for.
pub(crate) enum PaletteAction {
    /// The text in the field changed.
    Query(String),
    /// Move the cursor by this many rows (the host clamps it).
    Move(i64),
    /// The wheel: move the rows in view by this many.
    Scroll(i64),
    /// Run the row at this slot (the lead's is 0 while there is one), or
    /// the cursor's (`None`); `true` keeps the palette open.
    Run(Option<usize>, bool),
}

impl Host {
    /// Open the palette in the main window, closing any open menu.
    pub(crate) fn open_palette(&mut self) {
        // The approval sheet waits for its answer first.
        if self.review_presented() {
            return;
        }
        let Some(shell) = self.shell.as_ref().map(ComponentHandle::clone_strong) else {
            return;
        };
        if self.menu_open {
            self.close_menu();
        }
        #[cfg(feature = "producer-audio")]
        self.core
            .physical_revoke(producer_input::RevokeReason::Menu);
        self.menu_open = true;
        self.menu_generation = self.shell_generation;
        self.menu_mode = MenuMode::Palette;
        self.palette = PaletteState {
            aim: true,
            ..PaletteState::default()
        };
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
        // The status strip yields: the first row shows the status.
        self.refresh_strip();
    }
    /// The commands the open palette lists: from the interface's copies
    /// of the tables (rule R8; `refresh_menu` took the newest).
    pub(crate) fn palette_items(&self) -> Vec<Item> {
        self.items_of(&self.registry.shown_entries(), &self.labels())
    }
    /// The commands the palette lists, from the core's own tables: what
    /// `commands.list` answers and `commands.run` checks against.
    pub(crate) fn listed_items(&self) -> Vec<Item> {
        let bindings = self.core.bindings();
        self.items_of(&self.core.commands.shown_entries(), &|owner, id| {
            bindings.label(owner, id)
        })
    }
    /// The palette's items of `entries`, with the keys `labels` gives.
    fn items_of(
        &self,
        entries: &[commands::CommandSnapshot],
        labels: &dyn Fn(u64, &str) -> Option<String>,
    ) -> Vec<Item> {
        let names = self.core.plugin_names();
        let name = |id: &str| {
            names
                .get(id)
                .cloned()
                .or_else(|| self.core.on_demand.name_of(id).map(str::to_owned))
                .unwrap_or_else(|| id.to_owned())
        };
        command_palette::items(entries, &name, labels)
    }
    /// The lead was set, changed or cleared (`palette.lead`): an open
    /// palette shows it now; `moved` (it came or went) places the cursor
    /// again.
    pub(crate) fn lead_changed(&mut self, moved: bool) {
        if self.menu_open && self.menu_mode == MenuMode::Palette {
            if moved {
                self.palette.aim = true;
            }
            let result = self.refresh_menu();
            self.report_menu(result);
        }
        self.lead_strip_follow();
    }
    /// Project the open palette into `window` (`refresh_menu`): the
    /// commands as they are now, matched against the query.
    pub(crate) fn refresh_palette(
        &mut self,
        window: &slint_interpreter::ComponentInstance,
    ) -> Result<()> {
        let items = self.palette_items();
        let hits =
            command_palette::search(&items, &self.palette.query, &self.remembered.palette_recent);
        let (_, size) = super::menus::menu_frame(window);
        let lead = self
            .core
            .lead
            .as_ref()
            .map(|lead| (lead.owner, lead.row.clone()));
        let leading = usize::from(lead.is_some());
        // The lead takes one of the rows in view.
        let visible = command_palette::visible(size.y)
            .saturating_sub(leading)
            .max(1);
        let empty = self.palette.query.trim().is_empty();
        let status = lead
            .as_ref()
            .and_then(|(_, row)| row.status.clone())
            .filter(|_| command_palette::lead_shows_status(true, &self.palette.query));
        let palette = &mut self.palette;
        // The status came or went with nothing typed (it came after
        // Shift+Enter cleared the field): the cursor goes where the rule
        // puts it, on the status row while there is one (Enter stops).
        palette.track_lead(lead.is_some(), status.is_some());
        if std::mem::take(&mut palette.aim) {
            palette.cursor = command_palette::aim(
                &items,
                &hits,
                &palette.query,
                palette.lead,
                status.is_some(),
            );
            palette.offset = 0;
        }
        if let Some(global) = palette.follow.take()
            && let Some(at) = command_palette::position(&items, &hits, &global)
        {
            palette.cursor = leading + at;
        }
        palette.hits = hits;
        palette.cursor = palette.cursor.min(palette.rows().saturating_sub(1));
        let count = palette.hits.len();
        palette.offset = match palette.command_at_cursor() {
            Some(at) => command_palette::reveal(palette.offset, at, visible, count),
            None => palette.offset.min(count.saturating_sub(visible)),
        };
        let rows = command_palette::rows(&items, &palette.hits, palette.offset, visible);
        let current = match palette.command_at_cursor() {
            _ if palette.rows() == 0 => -1,
            None => 0,
            Some(at) => (leading + at - palette.offset) as i64,
        };
        let shown = count.saturating_sub(palette.offset).min(visible);
        let results = i18n::tr_args("Results: {}", &json!([count]));
        let placeholder = lead
            .as_ref()
            .and_then(|(_, row)| row.placeholder.clone())
            .unwrap_or_default();
        palette.items = items;
        palette.visible = visible;
        let lead_row = match &lead {
            None => json!({"shown": false, "icon": "", "title": "", "query": "",
                "empty": "", "note": "", "status": false}),
            Some((_, row)) => match &status {
                Some(status) => json!({"shown": true, "icon": lead_icon(&row.icon),
                    "title": status.text, "query": "", "empty": "",
                    "note": status.note.clone().unwrap_or_default(), "status": true}),
                None => json!({"shown": true, "icon": lead_icon(&row.icon),
                    "title": row.title, "query": self.palette.query,
                    "empty": if empty { row.empty.clone().unwrap_or_default() } else { String::new() },
                    "note": row.note.clone().unwrap_or_default(), "status": false}),
            },
        };
        let image = match &lead {
            Some((owner, row)) => self.lead_image(*owner, &row.icon),
            None => None,
        };
        let next = ui_values::from_json(&rows)?;
        let live = window.get_property("palette-rows")?;
        if !ui_values::reconcile_nested(&live, &next)? {
            window.set_property("palette-rows", next)?;
        }
        window.set_property("palette-current", SValue::Number(current as f64))?;
        window.set_property("palette-shown", SValue::Number(shown as f64))?;
        window.set_property("palette-count", SValue::String(results.as_str().into()))?;
        let mut lead_value = ui_values::from_json(&lead_row)?;
        if let SValue::Struct(fields) = &mut lead_value {
            fields.set_field("image".into(), SValue::Image(image.unwrap_or_default()));
        }
        window.set_property("palette-lead", lead_value)?;
        window.set_property(
            "palette-placeholder",
            SValue::String(placeholder.as_str().into()),
        )?;
        Ok(())
    }
    /// A lead's image icon, loaded for the row's icon box (the "failed"
    /// icon shows when it cannot be read).
    pub(crate) fn lead_image(
        &mut self,
        owner: u64,
        icon: &commands::ButtonIcon,
    ) -> Option<slint::Image> {
        let commands::ButtonIcon::Image { path, max_side } = icon else {
            self.images.assign(owner, "$palette", "lead", None);
            return None;
        };
        let side = if *max_side == 0 {
            64
        } else {
            (*max_side).min(64)
        };
        let roots = self.core.image_roots(owner);
        match self.images.load_within(owner, &roots, path, side) {
            Ok((key, image)) => {
                self.images.assign(owner, "$palette", "lead", Some(key));
                Some(image)
            }
            Err(error) => {
                diag!("palette lead: {owner}: {}", error.message);
                None
            }
        }
    }
    /// The lead's row was activated (Enter, Shift+Enter, a click): its
    /// holder gets the query and a new ticket (`palette.lead` event, which
    /// wakes it from standby). `status`: the row showed its status (stop).
    fn run_lead(&mut self, keep: bool) {
        let query = self.palette.query.clone();
        let has_status = self
            .core
            .lead
            .as_ref()
            .is_some_and(|lead| lead.row.status.is_some());
        // Nothing typed after Shift+Enter: Enter stops that request, also
        // before its status arrived.
        let status = command_palette::lead_shows_status(has_status || self.palette.sent, &query);
        self.lead_request(query, keep, status);
        if status {
            self.palette.sent = false;
        }
        // Kept open (Shift+Enter): the field is cleared, so the row shows
        // the holder's status for the request it just sent, and a second
        // Enter stops it instead of sending it again (PROPOSAL §4.6, F1).
        if keep && !self.palette.query.is_empty() {
            self.palette.kept_sent();
            if let Some(shell) = &self.shell
                && let Err(error) = shell.set_property("palette-query", SValue::String("".into()))
            {
                diag!("host.error palette: {error}");
            }
        }
    }
    /// The lead's row activated with `query` (also `VESTRA_TEST_EXECUTE`'s
    /// `lead:<query>@<ms>`, headless): the row showed the holder's status
    /// when it has one and nothing is typed.
    pub(crate) fn lead_activated(&mut self, query: String, keep: bool) {
        let Some(lead) = &self.core.lead else {
            return;
        };
        let status = command_palette::lead_shows_status(lead.row.status.is_some(), &query);
        self.lead_request(query, keep, status);
    }
    /// The interface's part of a request to the lead's holder (plan step
    /// S2f(1)): the core sends it (`UiInput::Lead`, `lead_event`); a menu
    /// that is not kept closes after it.
    pub(crate) fn lead_request(&mut self, query: String, keep: bool, status: bool) {
        if self.core.lead.is_none() {
            return;
        }
        self.ui_report(messages::UiReport::Input(messages::UiInput::Lead {
            query,
            keep,
            status,
        }));
        if !keep && self.menu_open {
            self.close_menu();
        }
    }
    /// The core's part of a palette row (`UiInput::Command`, plan step
    /// S2f(1)): the window it came from is recorded, the command runs.
    pub(crate) fn palette_command(
        &mut self,
        owner: u64,
        id: &str,
        origin: Option<origins::Origin>,
    ) {
        if let Some(origin) = origin {
            self.core.act(owner, origin);
        }
        if let Err(error) = self.execute_command(owner, id) {
            diag!("palette execute: {:?}", error);
        }
    }
    /// The `palette.lead` event to the holder, with a new ticket; `status`:
    /// the user activated the status (Enter on the status row, the strip's
    /// Stop), which stops the running request.
    pub(crate) fn lead_event(&mut self, query: String, keep: bool, status: bool) {
        let Some(lead) = &self.core.lead else {
            return;
        };
        let owner = lead.owner;
        // The palette's first row and the status strip's Stop are the main
        // window's: the holder's confirmations for this request (Ask
        // Vestra's consent, the commands it runs) go to its sheet, with the
        // request box, even after the holder's own window was shown.
        self.act(owner, origins::Origin::Lead);
        let now = self.core.now_ms();
        let ticket = self.core.command_runs.mint(owner, &query, now);
        // A new request: its end state shows even when it is the same as
        // the last one's (the same failure twice in a row).
        if let Some(lead) = self.core.lead.as_mut() {
            lead.ended = None;
        }
        input_trace::follow(&format!("palette lead to plugin {owner} (keep {keep})"));
        // A menu not kept closes after this line, on the interface's side
        // (`lead_request`; before plan step S2f(1) it closed before it).
        self.send(
            owner,
            json!({"event": crate::capabilities::LEAD_EVENT,
                "data": {"query": query, "keep": keep, "ticket": ticket, "status": status}}),
        );
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
                self.palette.sent = false;
                self.palette.cursor = 0;
                self.palette.offset = 0;
                self.palette.aim = true;
            }
            PaletteAction::Move(by) => {
                let count = self.palette.rows();
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
                // The cursor stays in view (the lead's row always is).
                if let Some(at) = self.palette.command_at_cursor() {
                    let at = at.clamp(offset, (offset + visible - 1).min(count.saturating_sub(1)));
                    self.palette.cursor = usize::from(self.palette.lead) + at;
                }
            }
            PaletteAction::Run(slot, keep) => {
                let leading = usize::from(self.palette.lead);
                // The row's place among the lead and `hits`.
                let row = match slot {
                    Some(0) if self.palette.lead => 0,
                    Some(slot) => leading + self.palette.offset + slot.saturating_sub(leading),
                    None => self.palette.cursor,
                };
                let Some(index) = row.checked_sub(leading) else {
                    self.run_lead(keep);
                    if !keep {
                        return;
                    }
                    let result = self.refresh_menu();
                    self.report_menu(result);
                    return;
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
                command_palette::remember(&mut self.remembered.palette_recent, &item.global);
                let now = self.core.now_ms();
                self.remember(now);
                // Kept open: the cursor stays on this command wherever it
                // now sorts (the recent list reorders an empty query).
                self.palette.cursor = row;
                self.palette.follow = Some(item.global.clone());
                if !keep {
                    self.close_menu();
                }
                // The palette is the main window's.
                self.ui_report(messages::UiReport::Input(messages::UiInput::Command {
                    from: messages::CommandFrom::Palette,
                    owner: item.owner,
                    id: item.id,
                    origin: Some(origins::Origin::Main),
                }));
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
        let events = self.core.events.clone();
        window.set_callback("palette-edited", move |args| {
            if let Some(SValue::String(text)) = args.first() {
                send(&events, PaletteAction::Query(text.to_string()));
            }
            SValue::Void
        })?;
        let events = self.core.events.clone();
        window.set_callback("palette-moved", move |args| {
            if let Some(SValue::Number(by)) = args.first()
                && by.is_finite()
            {
                send(&events, PaletteAction::Move(*by as i64));
            }
            SValue::Void
        })?;
        let events = self.core.events.clone();
        window.set_callback("palette-scrolled", move |args| {
            if let Some(SValue::Number(by)) = args.first()
                && by.is_finite()
            {
                send(&events, PaletteAction::Scroll(*by as i64));
            }
            SValue::Void
        })?;
        let events = self.core.events.clone();
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
impl Core {
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
}

/// The icon-set name a lead row draws (an image icon draws its image).
fn lead_icon(icon: &commands::ButtonIcon) -> String {
    match icon {
        commands::ButtonIcon::Name(name) => name.clone(),
        commands::ButtonIcon::Image { .. } => String::new(),
    }
}

/// The palette's own command id (`host/palette`).
pub(crate) const PALETTE: &str = "palette";

#[cfg(test)]
mod tests {
    use super::*;

    /// A palette showing a lead without a status, the cursor on the first
    /// command, the query typed.
    fn typed() -> PaletteState {
        PaletteState {
            query: "mute and go fullscreen".into(),
            cursor: 1,
            lead: true,
            ..PaletteState::default()
        }
    }

    #[test]
    fn after_shift_enter_the_cursor_stays_on_the_lead_until_the_status_comes() {
        let mut palette = typed();
        palette.kept_sent();
        assert_eq!((palette.query.as_str(), palette.cursor), ("", 0));
        assert!(palette.sent);
        // The next projection, before the holder's status arrived (a parked
        // holder waking): the cursor is not placed on the recent command.
        palette.track_lead(true, false);
        assert!(!palette.aim);
        assert_eq!(palette.cursor, 0);
        // Enter on the lead row now stops the request just sent.
        assert!(command_palette::lead_shows_status(
            palette.sent,
            &palette.query
        ));
        // The status comes: the cursor is placed again (onto it).
        palette.track_lead(true, true);
        assert!(palette.aim && palette.sent);
        assert_eq!(command_palette::aim(&[], &[], "", true, true), 0);
        // It goes (the request ended): Enter sends a new request again.
        palette.aim = false;
        palette.track_lead(true, false);
        assert!(palette.aim && !palette.sent);
    }

    #[test]
    fn a_status_change_while_typing_leaves_the_cursor_alone() {
        let mut palette = typed();
        palette.track_lead(true, true);
        assert!(!palette.aim);
        // The lead going away re-aims whatever is typed.
        palette.track_lead(false, true);
        assert!(palette.aim);
    }
}
