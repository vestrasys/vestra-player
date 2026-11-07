//! Bottom-bar command buttons (player 0.5.10, SDK 1.5.29). A command declared
//! with a `button` (`CommandOptions.button`: an icon and an order) and a menu
//! path gets a cell in the player's strip. Since the menu redesign (step 3a,
//! SDK 1.6.8) the player places the strip itself (`HostBarStrip`,
//! ui/bar-strip.slint) at the right end of the bottom slot, or in a band of
//! its own without a bottom bar: `[plugin buttons…][»][Command Palette]
//! [Adjust][Toggle Sidebar]` (Adjust: step 3b). The bar says how wide its row is with every control
//! (`content-full-width`; plugin buttons get only the room beyond it) or at
//! least (`content-min-width`, a 3a bar; composition.rs) and reads the strip's width
//! (`BarButtonsHost.width`). A bar that still places the deprecated
//! `BarButtons` gets that one instead (the player's hides itself).
//!
//! The player fills the strip's global (`BarButtonsHost`) in every window it
//! composes and runs a click like a click on the command's menu item
//! (`execute_command`). One button per plugin, eight in the strip
//! (`commands::Registry::buttons`); the rest, and the ones the width leaves
//! out, are in the strip's `»` menu (the host menu layer, `MenuMode::Bar`).
//! Nothing is left out silently, so the remaining log lines are debug
//! (`VESTRA_DEBUG_BAR=1`).
#![forbid(unsafe_code)]
use crate::host::*;
use crate::ui::{composition, window_state};
use crate::{bindings, commands, i18n, input_trace};
use anyhow::Result;
use serde_json::json;
use slint::ComponentHandle;
use slint_interpreter::Value as SValue;
use std::collections::BTreeSet;

/// The global every composed window exports.
pub const GLOBAL: &str = "BarButtonsHost";
/// The player's Toggle Sidebar command (owner `bindings::HOST`).
pub const SIDEBAR: &str = "sidebar";
/// The player's Adjust command (owner `bindings::HOST`, menu redesign step
/// 3b): shows or hides the adjust column.
pub const ADJUST: &str = "adjust";

/// `VESTRA_DEBUG_BAR=1`: the strip's debug lines (buttons in `»`, a
/// plugin's buttons after its first, a bar placing `BarButtons`).
fn debug_bar() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VESTRA_DEBUG_BAR").is_some_and(|v| v == "1"))
}

/// One cell as the player fills it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub owner: u64,
    pub id: String,
    pub icon: commands::ButtonIcon,
    /// The shown title without a trailing "…" (the accessibility label).
    pub label: String,
    pub tip: String,
    pub enabled: bool,
    pub checkable: bool,
    pub checked: bool,
    pub dimmed: bool,
}

impl Item {
    /// What the cell's click sends back.
    fn token(&self) -> String {
        format!("{}/{}", self.owner, self.id)
    }
}

/// A button not in the strip: owner, command id, plugin id, where it went.
type Dropped = (u64, String, String, &'static str);

/// What the strip is computed from. The window tick compares it (a few
/// integers) and rebuilds only when it changed (UI frame-rate rule): the
/// registry's revision (commands, their state and buttons), the binding
/// table's (the keys in tooltips), the sidebar's state (the toggle), the
/// interface language, and the composed windows (a new shell or floating
/// window gets the strip and may place `BarButtons` or not).
#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    pub commands: u64,
    pub bindings: u64,
    pub sidebar: Option<window_state::Visibility>,
    pub slot: Option<(bool, bool)>,
    /// The adjust column's state (its toggle's dot).
    pub adjust: Option<(bool, bool)>,
    pub locale: &'static str,
    pub windows: (u64, Option<u64>),
}

/// What the windows show, and what was logged.
#[derive(Default)]
pub struct Strip {
    /// What the strip was last built from.
    inputs: Option<Inputs>,
    /// Rebuilds so far (a test counts them).
    rebuilds: u64,
    buttons: Vec<Item>,
    /// Past [`commands::Registry::MAX_BUTTONS`]: always in `»`.
    overflow: Vec<Item>,
    toggle: Option<Item>,
    palette: Option<Item>,
    adjust: Option<Item>,
    /// Window generations that show `buttons`, `overflow`, `toggle`,
    /// `palette`, `adjust`.
    applied: BTreeSet<u64>,
    /// Buttons not in the strip, logged once each (debug).
    logged: BTreeSet<(u64, String)>,
    /// The shell generation whose bar was seen placing `BarButtons` (debug).
    placed: Option<u64>,
}

impl Strip {
    /// What `»` lists, in strip order: the plugin buttons from `shown` on
    /// (the ones the room leaves out), then the ones past eight.
    pub fn more(&self, shown: usize) -> Vec<(u64, String)> {
        self.buttons
            .iter()
            .skip(shown)
            .chain(&self.overflow)
            .map(|item| (item.owner, item.id.clone()))
            .collect()
    }
    /// Whether the strip must be rebuilt for `inputs`; counts the rebuilds.
    fn begin(&mut self, inputs: &Inputs) -> bool {
        if self.inputs.as_ref() == Some(inputs) {
            return false;
        }
        self.rebuilds += 1;
        true
    }
    /// The command behind a click's token, if it is in the strip's list
    /// (the plugins' buttons and the toggle). A cell hidden for want of room
    /// is not clickable in the first place: its TouchArea is disabled and
    /// its accessibility action guarded.
    pub fn target(&self, token: &str) -> Option<(u64, String)> {
        self.buttons
            .iter()
            .chain(self.toggle.as_ref())
            .chain(self.palette.as_ref())
            .chain(self.adjust.as_ref())
            .find(|item| item.token() == token)
            .map(|item| (item.owner, item.id.clone()))
    }
}

/// A menu title as a button names it: without a trailing "…" or "...".
pub fn label(title: &str) -> String {
    let title = title.trim_end();
    title
        .strip_suffix('…')
        .or_else(|| title.strip_suffix("..."))
        .unwrap_or(title)
        .trim_end()
        .to_owned()
}

/// The tooltip: the label and the command's first key, as the bar's own
/// buttons name them ("Plugin Store (F9)"), then, while it cannot run, why.
pub fn tip(label: &str, key: Option<&str>, state: &commands::CommandState) -> String {
    let named = match key {
        Some(key) => i18n::tr_args("{} ({})", &json!([label, key])),
        None => label.to_owned(),
    };
    match &state.reason {
        Some(reason) if !state.enabled || state.busy => format!("{named}\n{reason}"),
        _ => named,
    }
}

/// The cell for a button whose image cannot be loaded: the `failed` icon.
fn fallback(item: &Item) -> Item {
    Item {
        icon: commands::ButtonIcon::Name("failed".into()),
        ..item.clone()
    }
}

/// The Slint value of one cell (`None`: no cell, an empty token).
pub fn cell(item: Option<&Item>, image: Option<slint::Image>) -> SValue {
    let icon = match item.map(|item| &item.icon) {
        Some(commands::ButtonIcon::Name(name)) => name.as_str(),
        _ => "",
    };
    let flag = |get: fn(&Item) -> bool| SValue::Bool(item.is_some_and(get));
    let text = |get: fn(&Item) -> &str| SValue::String(item.map_or("", get).into());
    [
        (
            "token",
            SValue::String(item.map(Item::token).unwrap_or_default().into()),
        ),
        ("icon", SValue::String(icon.into())),
        ("image", SValue::Image(image.unwrap_or_default())),
        ("label", text(|item| &item.label)),
        ("tip", text(|item| &item.tip)),
        ("enabled", flag(|item| item.enabled)),
        ("checkable", flag(|item| item.checkable)),
        ("checked", flag(|item| item.checked)),
        ("dimmed", flag(|item| item.dimmed)),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect::<slint_interpreter::Struct>()
    .into()
}

impl Host {
    /// Toggle Sidebar: show or hide the sidebar, as `panels.sidebar.toggle()`
    /// does (the preference of its current panel).
    pub(crate) fn toggle_sidebar(&mut self) {
        let (Some(owner), Some((preferred, _))) =
            (self.policy.sidebar_active(), self.policy.sidebar_state())
        else {
            return;
        };
        let now = self.now_ms();
        if self
            .policy
            .set_preferred(window_state::Slot::Sidebar, owner, !preferred, now)
            .is_ok()
        {
            if let Some(mount) = self.mounts.get_mut(&owner) {
                mount.visible = !preferred;
            }
            self.remember_sidebar(false);
            self.window_tick();
        }
    }

    /// Adjust (`host/adjust`): show or hide the adjust column, showing its
    /// last tab, as `panels.adjust.toggle()` does.
    pub(crate) fn toggle_adjust(&mut self) {
        if let Some((preferred, _)) = self.policy.column_state(window_state::Slot::Adjust) {
            self.set_adjust_visible(!preferred);
        }
    }

    /// Show or hide the adjust column (the preference of its current panel).
    pub(crate) fn set_adjust_visible(&mut self, visible: bool) {
        let Some(owner) = self.policy.active(window_state::Slot::Adjust) else {
            return;
        };
        let now = self.now_ms();
        if self
            .policy
            .set_preferred(window_state::Slot::Adjust, owner, visible, now)
            .is_ok()
        {
            self.remember_column(crate::ui::sidebar_tabs::Column::Adjust, false);
            self.window_tick();
        }
    }

    /// Toggle Sidebar's and Adjust's states: available while a panel is in
    /// their column, checked while it is wanted shown.
    fn sidebar_command_state(&mut self) {
        for (id, slot) in [
            (SIDEBAR, window_state::Slot::Sidebar),
            (ADJUST, window_state::Slot::Adjust),
        ] {
            let preferred = self
                .policy
                .column_state(slot)
                .map(|(preferred, _)| preferred);
            let state = commands::CommandState {
                enabled: preferred.is_some(),
                checked: Some(preferred.unwrap_or(false)),
                busy: false,
                reason: preferred.is_none().then(|| match slot {
                    window_state::Slot::Sidebar => i18n::tr("No panel is in the sidebar"),
                    _ => i18n::tr("No panel is in the adjust column"),
                }),
            };
            if self
                .commands
                .get(bindings::HOST, id)
                .is_some_and(|entry| entry.state != state)
            {
                self.host_revision += 1;
                let _ = self
                    .commands
                    .update(bindings::HOST, id, self.host_revision, state);
            }
        }
    }

    /// The plugins' cells (the strip's, then the ones past eight), and the
    /// buttons not in the strip (owner, id, plugin id, where they went).
    fn bar_items(&self) -> (Vec<Item>, Vec<Item>, Vec<Dropped>) {
        let buttons = self.commands.buttons();
        let input = self.input.borrow();
        let items = |list: &[&commands::CommandSnapshot]| -> Vec<Item> {
            list.iter()
                .filter_map(|entry| {
                    let button = entry.declaration.button.as_ref()?;
                    let (owner, id) = (entry.key.owner, entry.key.local_id.clone());
                    let label = label(entry.declaration.shown_title());
                    let key = input.table.label(owner, &id);
                    Some(Item {
                        tip: tip(&label, key.as_deref(), &entry.state),
                        owner,
                        id,
                        icon: button.icon.clone(),
                        label,
                        enabled: entry.state.enabled && !entry.state.busy,
                        checkable: entry.state.checked.is_some(),
                        checked: entry.state.checked == Some(true),
                        dimmed: false,
                    })
                })
                .collect()
        };
        let (shown, overflow) = (items(&buttons.shown), items(&buttons.overflow));
        let dropped = buttons
            .dropped
            .iter()
            .copied()
            .chain(
                buttons
                    .overflow
                    .iter()
                    .map(|entry| (*entry, "past 8 in the strip: in its » menu")),
            )
            .map(|(entry, limit)| {
                (
                    entry.key.owner,
                    entry.key.local_id.clone(),
                    entry.manifest.clone(),
                    limit,
                )
            })
            .collect();
        (shown, overflow, dropped)
    }

    /// The command palette's cell (`host/palette`): the magnifier, always
    /// shown, right of `»`.
    fn palette_item(&self) -> Option<Item> {
        let entry = self.commands.get(bindings::HOST, palette::PALETTE)?;
        let label = label(entry.declaration.shown_title());
        let key = self
            .input
            .borrow()
            .table
            .label(bindings::HOST, palette::PALETTE);
        Some(Item {
            owner: bindings::HOST,
            id: palette::PALETTE.into(),
            icon: commands::ButtonIcon::Name("search".into()),
            tip: tip(&label, key.as_deref(), &entry.state),
            label,
            enabled: entry.state.enabled && !entry.state.busy,
            checkable: false,
            checked: false,
            dimmed: false,
        })
    }

    /// The sidebar toggle's cell: the Toggle Sidebar command, the playlist
    /// icon, its on dot while the sidebar is wanted, at half opacity while it is
    /// wanted but the window is too narrow for it (G-18).
    fn sidebar_item(&self, sidebar: Option<window_state::Visibility>) -> Option<Item> {
        let entry = self.commands.get(bindings::HOST, SIDEBAR)?;
        let label = label(entry.declaration.shown_title());
        let key = self.input.borrow().table.label(bindings::HOST, SIDEBAR);
        let dimmed = sidebar.is_some_and(|s| {
            s.preferred && !s.effective && s.reason == Some(window_state::Reason::Space)
        });
        let mut tip = tip(&label, key.as_deref(), &entry.state);
        if dimmed {
            tip = i18n::tr_args(
                "{} (the window is too narrow for the sidebar)",
                &json!([tip]),
            );
        }
        Some(Item {
            owner: bindings::HOST,
            id: SIDEBAR.into(),
            icon: commands::ButtonIcon::Name("playlist".into()),
            label,
            tip,
            enabled: entry.state.enabled,
            checkable: true,
            checked: entry.state.checked == Some(true),
            dimmed,
        })
    }

    /// The adjust column's cell (`host/adjust`, menu redesign step 3b): the
    /// sliders icon, its on dot while the column is wanted shown; disabled
    /// while no panel is in it.
    fn adjust_item(&self) -> Option<Item> {
        let entry = self.commands.get(bindings::HOST, ADJUST)?;
        let label = label(entry.declaration.shown_title());
        let key = self.input.borrow().table.label(bindings::HOST, ADJUST);
        Some(Item {
            owner: bindings::HOST,
            id: ADJUST.into(),
            icon: commands::ButtonIcon::Name("sliders".into()),
            tip: tip(&label, key.as_deref(), &entry.state),
            label,
            enabled: entry.state.enabled,
            checkable: true,
            checked: entry.state.checked == Some(true),
            dimmed: false,
        })
    }

    /// Each window tick: bring the strip of every composed window up to
    /// date (`columns`: the sidebar's and the adjust column's states at
    /// this tick).
    pub(crate) fn bar_tick(
        &mut self,
        shell: &slint_interpreter::ComponentInstance,
        columns: (
            Option<window_state::Visibility>,
            Option<window_state::Visibility>,
        ),
    ) {
        let sidebar = columns.0;
        if !self.bar.begin(&self.bar_inputs(sidebar)) {
            return;
        }
        self.sidebar_command_state();
        let (buttons, overflow, dropped) = self.bar_items();
        for (owner, id, manifest, limit) in dropped {
            if self.bar.logged.insert((owner, id.clone())) && debug_bar() {
                diag!("bar buttons: {manifest}/{id}: {limit}; its menu entry stays");
            }
        }
        let toggle = self.sidebar_item(sidebar);
        let palette = self.palette_item();
        let adjust = self.adjust_item();
        if buttons != self.bar.buttons
            || overflow != self.bar.overflow
            || toggle != self.bar.toggle
            || palette != self.bar.palette
            || adjust != self.bar.adjust
        {
            for item in &self.bar.buttons {
                self.images.release_view(item.owner, "$bar");
            }
            self.bar.buttons = buttons;
            self.bar.overflow = overflow;
            self.bar.toggle = toggle;
            self.bar.palette = palette;
            self.bar.adjust = adjust;
            self.bar.applied.clear();
        }
        let mut windows = vec![(self.shell_generation, shell.clone_strong())];
        if let Some(floating) = &self.floating {
            windows.push((floating.generation, floating.instance.clone_strong()));
        }
        self.bar
            .applied
            .retain(|generation| windows.iter().any(|(live, _)| live == generation));
        for (generation, window) in &windows {
            if self.bar.applied.insert(*generation)
                && let Err(error) = self.apply_bar(window, *generation)
            {
                diag!("bar buttons: {error:#}");
            }
        }
        self.bar_placement(shell);
        // After Toggle Sidebar's own state update, which moves the revision.
        self.bar.inputs = Some(self.bar_inputs(sidebar));
    }

    /// What the strip depends on, at this tick.
    fn bar_inputs(&self, sidebar: Option<window_state::Visibility>) -> Inputs {
        Inputs {
            commands: self.commands.revision(),
            bindings: self.input.borrow().table.revision(),
            sidebar,
            slot: self.policy.sidebar_state(),
            adjust: self.policy.column_state(window_state::Slot::Adjust),
            locale: i18n::locale(),
            windows: (
                self.shell_generation,
                self.floating.as_ref().map(|floating| floating.generation),
            ),
        }
    }

    /// Fill one window's `BarButtonsHost` and route its clicks.
    fn apply_bar(
        &mut self,
        window: &slint_interpreter::ComponentInstance,
        generation: u64,
    ) -> Result<()> {
        let events = self.events.clone();
        window.set_global_callback(GLOBAL, "clicked", move |args| {
            if let Some(SValue::String(token)) = args.first() {
                let _ = events.send(Event::BarButton(generation, token.to_string()));
            }
            SValue::Void
        })?;
        let mut cells = Vec::new();
        for item in self.bar.buttons.clone() {
            let image = match &item.icon {
                commands::ButtonIcon::Image { path, max_side } => {
                    // Drawn in the 14×12 icon box: decoded at most 64 px.
                    let side = if *max_side == 0 {
                        64
                    } else {
                        (*max_side).min(64)
                    };
                    let roots = self.image_roots(item.owner);
                    match self.images.load_within(item.owner, &roots, path, side) {
                        Ok((key, image)) => {
                            self.images.assign(item.owner, "$bar", &item.id, Some(key));
                            Some(image)
                        }
                        Err(error) => {
                            // The cell keeps an icon (and stays the command's
                            // button): the icon set's warning triangle.
                            diag!(
                                "bar buttons: {}/{}: {}; shown with the \"failed\" icon",
                                item.owner,
                                item.id,
                                error.message
                            );
                            cells.push(cell(Some(&fallback(&item)), None));
                            continue;
                        }
                    }
                }
                commands::ButtonIcon::Name(_) => None,
            };
            cells.push(cell(Some(&item), image));
        }
        window.set_global_property(
            GLOBAL,
            "buttons",
            SValue::Model(slint::ModelRc::new(slint::VecModel::from(cells))),
        )?;
        window.set_global_property(GLOBAL, "toggle", cell(self.bar.toggle.as_ref(), None))?;
        window.set_global_property(GLOBAL, "palette", cell(self.bar.palette.as_ref(), None))?;
        window.set_global_property(GLOBAL, "adjust", cell(self.bar.adjust.as_ref(), None))?;
        window.set_global_property(
            GLOBAL,
            "overflow",
            SValue::Number(self.bar.overflow.len() as f64),
        )?;
        Ok(())
    }

    /// A bottom bar that still places the deprecated `BarButtons`: the
    /// player's strip hides itself (no second strip); said once per shell
    /// (debug).
    fn bar_placement(&mut self, shell: &slint_interpreter::ComponentInstance) {
        let placed = matches!(
            shell.get_global_property(GLOBAL, "placed"),
            Ok(SValue::Bool(true))
        );
        if placed && self.bar.placed != Some(self.shell_generation) {
            self.bar.placed = Some(self.shell_generation);
            if debug_bar() {
                let bar = self
                    .mounts
                    .values()
                    .find(|mount| mount.spec.slot == composition::Slot::Bottom)
                    .map_or("?", |mount| mount.manifest.as_str());
                diag!(
                    "bar buttons: the bottom bar of {bar} places BarButtons (deprecated); the player's strip stays hidden"
                );
            }
        }
    }

    /// `»` in the strip of window `generation`, its top-left corner at
    /// (`x`, `y`): the plugin buttons not in the strip, as a host menu
    /// above it.
    pub(crate) fn bar_more(&mut self, generation: u64, x: f64, y: f64) {
        if !self.window_generation_live(generation) || !x.is_finite() || !y.is_finite() {
            return;
        }
        let Some(window) = self
            .window_for(generation)
            .map(ComponentHandle::clone_strong)
        else {
            return;
        };
        let shown = match window.get_global_property(GLOBAL, "shown") {
            Ok(SValue::Number(shown)) if shown >= 0. => shown as usize,
            _ => 0,
        };
        let items = self.bar.more(shown);
        if items.is_empty() {
            return;
        }
        input_trace::mark("host loop: bar button »");
        self.policy.activity(self.now_ms());
        let _ = window.set_property("menu-origin-x", x.into());
        let _ = window.set_property("menu-origin-y", y.into());
        self.open_menu(generation, MenuMode::Bar(items));
    }

    /// A click on a cell: run the command as its menu item would.
    pub(crate) fn bar_click(&mut self, generation: u64, token: &str) {
        if !self.window_generation_live(generation) {
            return;
        }
        let Some((owner, id)) = self.bar.target(token) else {
            return;
        };
        input_trace::mark(&format!("host loop: bar button → {owner}/{id}"));
        self.policy.activity(self.now_ms());
        if let Err(error) = self.execute_command(owner, &id) {
            diag!("bar button execute: {error:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(enabled: bool, busy: bool, reason: Option<&str>) -> commands::CommandState {
        commands::CommandState {
            enabled,
            checked: None,
            busy,
            reason: reason.map(str::to_owned),
        }
    }

    #[test]
    fn labels_drop_the_ellipsis_and_tips_name_the_key_and_the_reason() {
        assert_eq!(label("Plugin Store…"), "Plugin Store");
        assert_eq!(label("Vestra Player AI..."), "Vestra Player AI");
        assert_eq!(label("Toggle Sidebar"), "Toggle Sidebar");
        let on = state(true, false, None);
        assert_eq!(tip("Plugin Store", Some("F9"), &on), "Plugin Store (F9)");
        assert_eq!(tip("Plugin Store", None, &on), "Plugin Store");
        // A reason shows only while the command cannot run.
        let off = state(false, false, Some("Offline"));
        assert_eq!(tip("Plugin Store", None, &off), "Plugin Store\nOffline");
        let busy = state(true, true, Some("Loading"));
        assert_eq!(tip("A", Some("F9"), &busy), "A (F9)\nLoading");
        assert_eq!(tip("A", None, &state(true, false, Some("x"))), "A");
    }

    #[test]
    fn tokens_name_only_the_strips_list() {
        let item = |owner, id: &str| Item {
            owner,
            id: id.into(),
            icon: commands::ButtonIcon::Name("store".into()),
            label: String::new(),
            tip: String::new(),
            enabled: true,
            checkable: false,
            checked: false,
            dimmed: false,
        };
        let strip = Strip {
            buttons: vec![item(7, "open")],
            toggle: Some(item(bindings::HOST, SIDEBAR)),
            ..Strip::default()
        };
        assert_eq!(strip.target("7/open"), Some((7, "open".into())));
        assert_eq!(strip.target("0/sidebar"), Some((0, SIDEBAR.into())));
        // Another command of the same plugin, or of the player, is not a cell.
        assert_eq!(strip.target("7/secret"), None);
        assert_eq!(strip.target("0/quit"), None);
        // Step 3a: the palette cell is one.
        let strip = Strip {
            palette: Some(item(bindings::HOST, palette::PALETTE)),
            ..strip
        };
        assert_eq!(strip.target("0/palette"), Some((0, "palette".into())));
        // Step 3b: and the adjust column's toggle.
        let strip = Strip {
            adjust: Some(item(bindings::HOST, ADJUST)),
            ..strip
        };
        assert_eq!(strip.target("0/adjust"), Some((0, ADJUST.into())));
    }

    /// Step 3a: `»` lists the strip's buttons the room left out, then the
    /// ones past eight, in strip order; nothing is left out.
    #[test]
    fn more_lists_what_the_room_left_out_then_what_is_past_eight() {
        let item = |owner, id: &str| Item {
            owner,
            id: id.into(),
            icon: commands::ButtonIcon::Name("ai".into()),
            label: String::new(),
            tip: String::new(),
            enabled: true,
            checkable: false,
            checked: false,
            dimmed: false,
        };
        let strip = Strip {
            buttons: (1..=8).map(|owner| item(owner, "b")).collect(),
            overflow: vec![item(9, "b"), item(10, "b")],
            ..Strip::default()
        };
        let owners = |shown| {
            strip
                .more(shown)
                .into_iter()
                .map(|(owner, _)| owner)
                .collect::<Vec<_>>()
        };
        assert_eq!(owners(8), [9, 10]);
        assert_eq!(owners(6), [7, 8, 9, 10]);
        assert_eq!(owners(0), (1..=10).collect::<Vec<_>>());
        let few = Strip {
            buttons: vec![item(1, "b")],
            ..Strip::default()
        };
        assert!(few.more(1).is_empty());
        // `shown` past the list (a stale read) lists only the overflow.
        assert_eq!(strip.more(20).len(), 2);
    }

    /// The window tick rebuilds the strip only when what it is made from
    /// changed (UI frame-rate rule): an unchanged tick costs a comparison.
    #[test]
    fn an_unchanged_tick_does_not_rebuild() {
        let inputs = Inputs {
            commands: 7,
            bindings: 3,
            sidebar: None,
            slot: Some((true, false)),
            adjust: None,
            locale: "en",
            windows: (2, None),
        };
        let mut strip = Strip::default();
        assert!(strip.begin(&inputs));
        strip.inputs = Some(inputs.clone());
        for _ in 0..100 {
            assert!(!strip.begin(&inputs));
        }
        assert_eq!(strip.rebuilds, 1);
        // Each input that changes rebuilds once.
        for changed in [
            Inputs {
                commands: 8,
                ..inputs.clone()
            },
            Inputs {
                bindings: 4,
                ..inputs.clone()
            },
            Inputs {
                slot: Some((false, false)),
                ..inputs.clone()
            },
            Inputs {
                windows: (2, Some(5)),
                ..inputs.clone()
            },
            // Step 3b: the adjust column opened (its toggle's dot).
            Inputs {
                adjust: Some((true, false)),
                ..inputs.clone()
            },
        ] {
            assert!(strip.begin(&changed));
            strip.inputs = Some(changed.clone());
            assert!(!strip.begin(&changed));
        }
        assert_eq!(strip.rebuilds, 6);
    }

    #[test]
    fn an_image_that_cannot_load_falls_back_to_the_failed_icon() {
        let item = Item {
            owner: 4,
            id: "open".into(),
            icon: commands::ButtonIcon::Image {
                path: "/gone.png".into(),
                max_side: 0,
            },
            label: "Open".into(),
            tip: "Open".into(),
            enabled: true,
            checkable: false,
            checked: false,
            dimmed: false,
        };
        let shown = fallback(&item);
        assert_eq!(shown.icon, commands::ButtonIcon::Name("failed".into()));
        assert_eq!((shown.token(), shown.enabled), (item.token(), true));
        assert!(commands::BUTTON_ICONS.contains(&"failed"));
    }
}
