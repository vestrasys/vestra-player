//! Which window a plugin's `dialogs.confirm` belongs to: the one where the
//! user last acted for that plugin (doc/architecture/palette-lead.md §2.3).
//!
//! A confirmation reaches the host after the user's click has travelled to
//! the plugin's process and back, sometimes after a download or a check, so
//! the keyboard focus at its arrival says little about where it was asked
//! from: a click in the main window during that time, or the player in the
//! background, sent the Plugin Store's Update to the main window's sheet,
//! under the store window (tool windows stay above the main window), and
//! the store's next click was refused `busy` (bug of 2026-11-12). The host
//! knows where each request came from, so that decides:
//!
//! - **What counts**: a UI callback of the plugin's window, of its settings
//!   page (the Options window) or of its panel (the main window, or the
//!   floating sidebar's window); its command run from a menu, the command
//!   palette, a key, a bottom-bar button or a message's button (the window
//!   the menu or key was in); a request typed in the palette's first row,
//!   or its Stop in the status strip, for the lead's holder (the main
//!   window: Ask Vestra's consent and its commands' confirmations go to the
//!   sheet with the request box even when its own window was shown before);
//!   and the plugin showing its own window (`ui.open`, `ui.show`): the user
//!   then looks there. The newest wins; a command another plugin runs
//!   (`commands.run`) takes the caller's, or none when the caller has none.
//!   Only a live or parked plugin's input counts ([`super::Host::act`]).
//! - **Where it goes**: a tool window (the plugin's window, Options, the
//!   floating sidebar) still shown gets a system dialog owned by it, brought
//!   to the front; the main window gets the sheet. A closed origin falls
//!   back to the plugin's own shown window, and with neither to the focus
//!   test (`platform::tool_window_focused`), as before.
//! - Keyed by owner generation: a parked plugin's entry moves to its woken
//!   generation (`Capability::rekey` of `ui.dialog`) and goes when it stops.
use slint::ComponentHandle;
use std::collections::BTreeMap;

/// Where the user last acted for a plugin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The main window: its menus, palette, keys, bottom bar, panels.
    Main,
    /// The main window's palette first row: a request typed there for the
    /// lead's holder, or its Stop in the status strip. Routed as [`Self::Main`];
    /// the holder's confirmation then shows that request in its request box
    /// (`capabilities/ui_dialog.rs`), which an action in its panel since
    /// then (plain `Main`) does not.
    Lead,
    /// The floating sidebar's window (its panels, menus and keys).
    Floating,
    /// The Options window (the plugin's settings page).
    Options,
    /// The plugin's own tool window (`ui.open` id).
    View(String),
}

/// Each plugin's (owner generation's) newest origin.
#[derive(Default)]
pub(crate) struct Origins(BTreeMap<u64, Origin>);

impl Origins {
    /// The user acted for `owner` in `origin`.
    pub(crate) fn acted(&mut self, owner: u64, origin: Origin) {
        self.0.insert(owner, origin);
    }
    pub(crate) fn get(&self, owner: u64) -> Option<&Origin> {
        self.0.get(&owner)
    }
    /// `to` acts on behalf of `from` (`commands.run`): `from`'s origin
    /// becomes `to`'s; without one, `to` has none either (an older origin
    /// of `to`'s own would name a window this run did not come from).
    pub(crate) fn inherit(&mut self, from: u64, to: u64) {
        match self.0.get(&from).cloned() {
            Some(origin) => self.0.insert(to, origin),
            None => self.0.remove(&to),
        };
    }
    /// A parked owner woke as generation `to`.
    pub(crate) fn rekey(&mut self, from: u64, to: u64) {
        if let Some(origin) = self.0.remove(&from) {
            self.0.insert(to, origin);
        }
    }
    pub(crate) fn forget(&mut self, owner: u64) {
        self.0.remove(&owner);
    }
}

/// A tool window a confirmation can be owned by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    Floating,
    Options,
    /// The plugin's window with this id.
    View(String),
}

/// Where a confirmation goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// The main window's sheet.
    Sheet,
    /// A system dialog owned by this tool window, brought to the front.
    Owned(Place),
    /// No origin and no window of the plugin's shown: the focus decides.
    Focus,
}

/// The route of a confirmation from a plugin whose newest origin is
/// `origin`; `shown` tells whether a tool window is shown now (exists and
/// is visible), `own` lists the plugin's own windows (ids; the last shown
/// one wins); `main` whether the main window exists.
pub(crate) fn route(
    origin: Option<&Origin>,
    main: bool,
    own: &[String],
    shown: impl Fn(&Place) -> bool,
) -> Route {
    let place = match origin {
        Some(Origin::Main | Origin::Lead) if main => return Route::Sheet,
        Some(Origin::Main | Origin::Lead) | None => None,
        Some(Origin::Floating) => Some(Place::Floating),
        Some(Origin::Options) => Some(Place::Options),
        Some(Origin::View(id)) => Some(Place::View(id.clone())),
    };
    if let Some(place) = place.filter(|place| shown(place)) {
        return Route::Owned(place);
    }
    // The origin's window is gone: the plugin's own window still shown.
    if let Some(id) = own
        .iter()
        .rev()
        .find(|id| shown(&Place::View((*id).clone())))
    {
        return Route::Owned(Place::View(id.clone()));
    }
    Route::Focus
}

/// The owner of a file dialog (`dialogs.open`, `media.sources.locate`) of a
/// plugin whose confirmation would go `route`: the tool window a
/// confirmation would be owned by; otherwise (`None`) the main window, so
/// the dialog is never an unowned window under a fullscreen or on-top
/// player. Unlike a confirmation, the focus does not decide: a file dialog
/// has no sheet, and the main window is where the player is.
pub(crate) fn dialog_place(route: Route) -> Option<Place> {
    match route {
        Route::Owned(place) => Some(place),
        Route::Sheet | Route::Focus => None,
    }
}

/// The origin of input in the window of `generation` (a menu, a key, a
/// bottom-bar button, a panel's callback): the main window's
/// (`shell`) or the floating sidebar's (`floating`); a stale one: none.
pub(crate) fn of_generation(generation: u64, shell: u64, floating: Option<u64>) -> Option<Origin> {
    if generation == shell {
        Some(Origin::Main)
    } else if floating == Some(generation) {
        Some(Origin::Floating)
    } else {
        None
    }
}

/// The origin of a UI callback named `view`: a settings page's
/// (`$settings:<page>`, `ui/settings_window.rs`) is the Options window's;
/// a tool window's is that window's while it is open (`open`).
pub(crate) fn of_view(view: &str, open: bool) -> Option<Origin> {
    if view.starts_with("$settings:") {
        Some(Origin::Options)
    } else if open {
        Some(Origin::View(view.to_owned()))
    } else {
        None
    }
}

impl super::Core {
    /// The user acted for `owner` (a plugin's owner generation) in
    /// `origin` (`UiReport::Acted`). Only a running plugin's or a parked
    /// one's (its entry moves to the generation it wakes as) counts: a
    /// stopped owner's, or no plugin's (the host's own commands, owner 0),
    /// records nothing.
    pub(crate) fn act(&mut self, owner: u64, origin: Origin) {
        if self.plugins.contains_key(&owner) || self.on_demand.held(owner).is_some() {
            self.origins.acted(owner, origin);
        }
    }
}

impl super::Host {
    /// The user acted for `owner` in `origin`: the interface tells the
    /// core (`UiReport::Acted`, plan step S2c(5)), which keeps the origins.
    pub(crate) fn act(&mut self, owner: u64, origin: Origin) {
        self.ui_report(super::messages::UiReport::Acted { owner, origin });
    }
    /// The user acted for `owner` in the window of `generation`
    /// ([`of_generation`]).
    pub(crate) fn acted_in(&mut self, owner: u64, generation: u64) {
        let floating = self.floating.as_ref().map(|floating| floating.generation);
        if let Some(origin) = of_generation(generation, self.shell_generation, floating) {
            self.act(owner, origin);
        }
    }
    /// A UI callback of `owner`'s window or settings page ([`of_view`]).
    pub(crate) fn acted_in_view(&mut self, owner: u64, view: &str) {
        let open = self.views.contains_key(&(owner, view.to_owned()));
        if let Some(origin) = of_view(view, open) {
            self.act(owner, origin);
        }
    }
    /// The component of a tool window `place` of `owner`, while it exists.
    pub(crate) fn place_window<'a>(
        &'a self,
        owner: u64,
        place: &Place,
    ) -> Option<&'a slint_interpreter::ComponentInstance> {
        match place {
            Place::Floating => self.floating.as_ref().map(|floating| &floating.instance),
            Place::Options => self.settings.window(),
            Place::View(id) => self
                .views
                .get(&(owner, id.clone()))
                .map(|view| &view.instance),
        }
    }
    /// Where `owner`'s confirmation goes now ([`route`]).
    pub(crate) fn confirm_route(&self, owner: u64) -> Route {
        let own: Vec<String> = self
            .views
            .keys()
            .filter(|(each, _)| *each == owner)
            .map(|(_, id)| id.clone())
            .collect();
        route(
            self.core.origins.get(owner),
            self.shell.is_some(),
            &own,
            |place| {
                self.place_window(owner, place)
                    .is_some_and(|instance| instance.window().is_visible())
            },
        )
    }
    /// The owner of `owner`'s file dialog now ([`dialog_place`]).
    pub(crate) fn dialog_place(&self, owner: u64) -> Option<Place> {
        dialog_place(self.confirm_route(owner))
    }
    /// A request was refused `busy`: the confirmation or file dialog the
    /// user has to answer first comes into view, so the wait is never
    /// somewhere the user cannot see. A sheet: its main window (to the
    /// front while the player is in the foreground, asking for attention
    /// otherwise). An open system dialog, a confirmation or a file dialog,
    /// whichever window owns it: the dialog itself
    /// (`platform::show_open_dialog`; on Windows its owner is disabled
    /// while it is open).
    pub(crate) fn show_waiting_confirmation(&self) {
        let foreground = crate::platform::app_in_foreground();
        if self.sheet_waiting() {
            if let Some(shell) = &self.shell {
                crate::ui::view::show_again(shell, foreground);
            }
        } else if self.core.dialog_busy {
            crate::platform::show_open_dialog(foreground);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STORE: u64 = 7;
    const PLAYLIST: u64 = 9;

    fn view(id: &str) -> Origin {
        Origin::View(id.into())
    }
    fn place(id: &str) -> Place {
        Place::View(id.into())
    }

    /// The bug of 2026-11-12: Update clicked in the store window. Where the
    /// focus is when the request arrives (the store, the main window, another
    /// application, after a download or a check) does not enter: the store
    /// window owns its dialog while it is shown.
    #[test]
    fn a_click_in_the_store_window_confirms_in_the_store_window() {
        let mut origins = Origins::default();
        // Tools ▸ Plugin Store… from the main window's menu, the store
        // window shown (`ui.show`), Update clicked there.
        origins.acted(STORE, Origin::Main);
        origins.acted(STORE, view("store"));
        origins.acted(STORE, view("store"));
        assert_eq!(
            route(origins.get(STORE), true, &["store".into()], |p| *p
                == place("store")),
            Route::Owned(place("store"))
        );
        // The store window closed meanwhile: nothing of the store is shown.
        assert_eq!(
            route(origins.get(STORE), true, &["store".into()], |_| false),
            Route::Focus
        );
    }

    /// Update, Install and Uninstall all come from callbacks of the store
    /// window: one origin, whatever the action; any number of requests later.
    #[test]
    fn every_store_action_keeps_the_store_window_as_origin() {
        let mut origins = Origins::default();
        for _action in [
            "update",
            "install",
            "uninstall",
            "replace",
            "installVersion",
        ] {
            origins.acted(STORE, view("store"));
            for _later in 0..3 {
                assert_eq!(
                    route(origins.get(STORE), true, &[], |p| *p == place("store")),
                    Route::Owned(place("store"))
                );
            }
        }
    }

    /// A command run from the main window (a menu, the palette, a key, a
    /// bottom-bar button), or a click in the plugin's panel there: the sheet,
    /// even while one of the plugin's tool windows has the focus.
    #[test]
    fn the_main_window_gets_the_sheet_only_for_its_own_requests() {
        let mut origins = Origins::default();
        origins.acted(PLAYLIST, Origin::Main);
        assert_eq!(
            route(origins.get(PLAYLIST), true, &["editor".into()], |_| true),
            Route::Sheet
        );
        // No main window yet: no sheet; nothing else of it shown.
        assert_eq!(
            route(origins.get(PLAYLIST), false, &[], |_| true),
            Route::Focus
        );
    }

    /// Options and the floating sidebar are tool windows too: a settings
    /// page's or a floating panel's request is owned by that window while
    /// it is shown, and falls back once it closed.
    #[test]
    fn options_and_the_floating_sidebar_own_their_requests() {
        let mut origins = Origins::default();
        origins.acted(STORE, Origin::Options);
        let all = |_: &Place| true;
        assert_eq!(
            route(origins.get(STORE), true, &[], all),
            Route::Owned(Place::Options)
        );
        assert_eq!(
            route(origins.get(STORE), true, &[], |_| false),
            Route::Focus
        );
        origins.acted(PLAYLIST, Origin::Floating);
        assert_eq!(
            route(origins.get(PLAYLIST), true, &[], all),
            Route::Owned(Place::Floating)
        );
        // Docked again (the floating window closed): the focus decides.
        assert_eq!(
            route(origins.get(PLAYLIST), true, &[], |p| *p != Place::Floating),
            Route::Focus
        );
    }

    /// The origin's window closed: the plugin's own window still shown
    /// (newest first) owns it; with none, the focus decides.
    #[test]
    fn a_closed_origin_falls_back_to_the_plugins_own_window_then_the_focus() {
        let mut origins = Origins::default();
        origins.acted(STORE, view("gone"));
        let own = ["first".to_owned(), "second".to_owned()];
        assert_eq!(
            route(origins.get(STORE), true, &own, |p| *p != place("gone")),
            Route::Owned(place("second"))
        );
        assert_eq!(
            route(origins.get(STORE), true, &own, |p| *p == place("first")),
            Route::Owned(place("first"))
        );
        assert_eq!(
            route(origins.get(STORE), true, &own, |_| false),
            Route::Focus
        );
        // No origin at all (a timer's request): the plugin's own shown
        // window, else the focus.
        assert_eq!(
            route(None, true, &own, |p| *p == place("first")),
            Route::Owned(place("first"))
        );
        assert_eq!(route(None, true, &[], |_| true), Route::Focus);
    }

    /// The newest action wins: the store opened from the main window's menu
    /// and then a click in its window; the other way round, a main-window
    /// action after the store window's.
    #[test]
    fn the_newest_origin_wins() {
        let mut origins = Origins::default();
        origins.acted(STORE, Origin::Main);
        origins.acted(STORE, view("store"));
        assert_eq!(
            route(origins.get(STORE), true, &[], |_| true),
            Route::Owned(place("store"))
        );
        origins.acted(STORE, Origin::Main);
        assert_eq!(route(origins.get(STORE), true, &[], |_| true), Route::Sheet);
    }

    /// What each input records: the store window's callbacks (`act`,
    /// `rowAct`, `installVersion`: Update, Install, Uninstall) name its view;
    /// a settings page's name `$settings:<page>` (Options); a callback of a
    /// window already closed records nothing; a menu, key, bottom-bar
    /// button or panel callback is the main window's or the floating
    /// sidebar's by its window generation, and a stale generation's nothing.
    #[test]
    fn each_input_records_its_window() {
        assert_eq!(of_view("store", true), Some(view("store")));
        assert_eq!(of_view("store", false), None);
        assert_eq!(
            of_view("$settings:vestra.store", false),
            Some(Origin::Options)
        );
        assert_eq!(of_view("$settings:repos", true), Some(Origin::Options));
        assert_eq!(of_generation(4, 4, None), Some(Origin::Main));
        assert_eq!(of_generation(4, 4, Some(5)), Some(Origin::Main));
        assert_eq!(of_generation(5, 4, Some(5)), Some(Origin::Floating));
        assert_eq!(of_generation(3, 4, Some(5)), None);
        assert_eq!(of_generation(5, 4, None), None);
    }

    /// A delay before the request (a download, a check, a busy plugin
    /// process) and anything the user does elsewhere meanwhile that is not
    /// an action for this plugin (a click in the main window, another
    /// plugin's window, another application) leave the origin as it was.
    #[test]
    fn a_delay_and_other_plugins_actions_do_not_move_the_origin() {
        let mut origins = Origins::default();
        origins.acted(STORE, view("store"));
        // Meanwhile: the playlist used from the main window, another
        // plugin's window clicked.
        origins.acted(PLAYLIST, Origin::Main);
        origins.acted(42, view("editor"));
        assert_eq!(
            route(origins.get(STORE), true, &["store".into()], |_| true),
            Route::Owned(place("store"))
        );
        assert_eq!(
            route(origins.get(PLAYLIST), true, &[], |_| true),
            Route::Sheet
        );
        assert_eq!(
            route(origins.get(42), true, &["editor".into()], |_| true),
            Route::Owned(place("editor"))
        );
    }

    /// Generations: a woken owner keeps its entry; a stopped one loses it;
    /// one plugin's origin never routes another's request.
    #[test]
    fn origins_follow_owner_generations() {
        let mut origins = Origins::default();
        origins.acted(STORE, view("store"));
        origins.acted(PLAYLIST, Origin::Main);
        origins.rekey(STORE, 70);
        assert_eq!(origins.get(STORE), None);
        assert_eq!(origins.get(70), Some(&view("store")));
        origins.forget(70);
        assert_eq!(origins.get(70), None);
        assert_eq!(origins.get(PLAYLIST), Some(&Origin::Main));
        assert_eq!(origins.get(1), None);
    }

    /// A command another plugin runs (`commands.run`) takes the caller's
    /// origin; a caller without one leaves the target with none, not with
    /// an older origin of its own (review 2).
    #[test]
    fn a_run_on_a_callers_behalf_takes_the_callers_origin_or_none() {
        let mut origins = Origins::default();
        origins.acted(STORE, view("store"));
        origins.inherit(STORE, PLAYLIST);
        assert_eq!(origins.get(PLAYLIST), Some(&view("store")));
        // The playlist's own window was the newest before this run.
        origins.acted(PLAYLIST, view("editor"));
        origins.inherit(42, PLAYLIST);
        assert_eq!(origins.get(PLAYLIST), None);
        assert_eq!(
            route(origins.get(PLAYLIST), true, &[], |_| true),
            Route::Focus
        );
        // A caller running its own command keeps what it has.
        origins.inherit(STORE, STORE);
        assert_eq!(origins.get(STORE), Some(&view("store")));
        origins.inherit(42, 42);
        assert_eq!(origins.get(42), None);
    }

    const ASK: u64 = 11;

    /// Review 1: Ask Vestra (the Workshop) has its own window. A request
    /// typed in the palette's first row after that window was shown or
    /// clicked is the main window's (`lead_event`): its first-use consent
    /// goes to the sheet (with the request box), not to a dialog on the
    /// Workshop's window.
    #[test]
    fn a_palette_request_after_the_holders_window_confirms_on_the_sheet() {
        let mut origins = Origins::default();
        origins.acted(ASK, view("ai")); // ui.show / a click in the AI window
        origins.acted(ASK, Origin::Lead); // palette.lead: the typed request
        assert_eq!(
            route(origins.get(ASK), true, &["ai".into()], |_| true),
            Route::Sheet
        );
    }

    /// Review 1: the commands Ask Vestra runs for that request confirm on
    /// the sheet too, also a plugin whose own window is open and was used
    /// last, and Ask Vestra's own commands.
    #[test]
    fn a_command_ask_vestra_runs_confirms_on_the_sheet() {
        let mut origins = Origins::default();
        origins.acted(ASK, view("ai"));
        origins.acted(STORE, view("store"));
        origins.acted(ASK, Origin::Lead);
        origins.inherit(ASK, STORE);
        assert_eq!(
            route(origins.get(STORE), true, &["store".into()], |_| true),
            Route::Sheet
        );
        origins.inherit(ASK, ASK);
        assert_eq!(
            route(origins.get(ASK), true, &["ai".into()], |_| true),
            Route::Sheet
        );
    }

    /// T5 (H5 review follow-up 4): the consent's request box belongs to a
    /// request typed in the palette's first row. Once the user acted for the
    /// holder elsewhere in the main window (its docked panel, while the
    /// palette's ticket is still live), its confirmation still goes to the
    /// sheet but is no longer that request's: no request box, and the old
    /// ticket's end does not cancel it (`capabilities/ui_dialog.rs`).
    #[test]
    fn only_a_request_in_the_first_row_makes_a_confirmation_that_requests() {
        let mut origins = Origins::default();
        origins.acted(ASK, Origin::Lead);
        assert_eq!(route(origins.get(ASK), true, &[], |_| true), Route::Sheet);
        assert_eq!(origins.get(ASK), Some(&Origin::Lead));
        origins.acted(ASK, Origin::Main); // a click in its docked panel
        assert_eq!(route(origins.get(ASK), true, &[], |_| true), Route::Sheet);
        assert_ne!(origins.get(ASK), Some(&Origin::Lead));
        // Without the main window: the focus decides, as for `Main`.
        origins.acted(ASK, Origin::Lead);
        assert_eq!(route(origins.get(ASK), false, &[], |_| false), Route::Focus);
        // Wiring: the request box asks for this origin.
        assert!(
            after(
                include_str!("../capabilities/ui_dialog.rs"),
                "let request = host",
                8
            )
            .contains("== Some(&Origin::Lead)")
        );
    }

    /// The lines from `anchor` on (its first occurrence), `lines` of them.
    fn after(source: &str, anchor: &str, lines: usize) -> String {
        let at = source
            .find(anchor)
            .unwrap_or_else(|| panic!("anchor not found: {anchor}"));
        source[at..]
            .lines()
            .take(lines)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Wiring: every input path records its origin, so a path that loses
    /// its call (or a new one without it) fails here. Each command run by
    /// a user's input records it just before `execute_command`; the ones
    /// that do not come from an input are listed with why.
    #[test]
    fn every_input_path_records_an_origin() {
        let sources: &[(&str, &str)] = &[
            ("host/handle.rs", include_str!("handle.rs")),
            ("host/palette.rs", include_str!("palette.rs")),
            ("host/actions.rs", include_str!("actions.rs")),
            ("host/mounts.rs", include_str!("mounts.rs")),
            ("ui/bar_buttons.rs", include_str!("../ui/bar_buttons.rs")),
            (
                "capabilities/commands_run.rs",
                include_str!("../capabilities/commands_run.rs"),
            ),
            (
                "capabilities/ui_window.rs",
                include_str!("../capabilities/ui_window.rs"),
            ),
            ("host/supervisor/mod.rs", include_str!("supervisor/mod.rs")),
            (
                "host/supervisor/probe.rs",
                include_str!("supervisor/probe.rs"),
            ),
        ];
        // Not a user's input: a trigger replayed to a woken plugin (its
        // click recorded when it came), and the test switch.
        let exempt = ["host/supervisor/mod.rs", "host/supervisor/probe.rs"];
        let records = ["acted_in(", ".act(", "origins.inherit("];
        let mut calls = 0;
        for (file, source) in sources {
            let lines: Vec<&str> = source.lines().collect();
            for (index, line) in lines.iter().enumerate() {
                if !line.contains("execute_command(") || line.contains("fn execute_command") {
                    continue;
                }
                calls += 1;
                if exempt.contains(file) {
                    continue;
                }
                let before = lines[index.saturating_sub(6)..index].join("\n");
                assert!(
                    records.iter().any(|record| before.contains(record)),
                    "{file}:{}: execute_command without an origin recorded before it",
                    index + 1
                );
            }
        }
        // Menu, key, palette, bottom bar, message button, commands.run,
        // supervisor replay, probe.
        assert_eq!(calls, 8, "a new execute_command call: decide its origin");
        let handle = include_str!("handle.rs");
        assert!(
            after(handle, "Event::UiCallback(owner, data) =>", 6).contains("acted_in_view(owner")
        );
        assert!(
            after(include_str!("mounts.rs"), "fn mount_callbacks", 30)
                .contains("acted_in(owner, event.shell_generation)")
        );
        assert!(
            after(include_str!("palette.rs"), "fn lead_event", 12)
                .contains("self.act(owner, origins::Origin::Lead)")
        );
        let window = include_str!("../capabilities/ui_window.rs");
        assert!(
            after(window, "fn open(", 60)
                .contains("host.act(reply.owner, crate::host::origins::Origin::View(")
        );
        assert!(
            after(window, "\"ui.show\" =>", 10)
                .contains("host.act(key.0, crate::host::origins::Origin::View(")
        );
    }

    /// The bug of 2026-11-15: VestraTube's Download in a fullscreen player
    /// opened an unowned Save dialog under the player. A file dialog is
    /// owned like a confirmation: the plugin's shown tool window the user
    /// acted in (or its own window still shown), else the main window,
    /// never nobody. Every origin, with its window shown or not, and with
    /// or without a main window.
    #[test]
    fn a_file_dialog_is_owned_by_the_origin_window_else_the_main_window() {
        const TUBE: u64 = 11;
        let origins_of = [
            None,
            Some(Origin::Main),
            Some(Origin::Lead),
            Some(Origin::Floating),
            Some(Origin::Options),
            Some(view("link")),
        ];
        for origin in &origins_of {
            for main in [true, false] {
                for shown_window in [true, false] {
                    for own in [&[][..], &["link".to_owned()][..]] {
                        let mut origins = Origins::default();
                        if let Some(origin) = origin {
                            origins.acted(TUBE, origin.clone());
                        }
                        let shown = |p: &Place| shown_window && own_or_origin(p, own, origin);
                        let owner = dialog_place(route(origins.get(TUBE), main, own, shown));
                        let expected = match origin {
                            Some(Origin::Main | Origin::Lead) if main => None,
                            Some(Origin::Floating) if shown_window => Some(Place::Floating),
                            Some(Origin::Options) if shown_window => Some(Place::Options),
                            Some(Origin::View(id)) if shown_window && own.contains(id) => {
                                Some(place(id))
                            }
                            _ if shown_window && !own.is_empty() => Some(place("link")),
                            _ => None,
                        };
                        assert_eq!(
                            owner, expected,
                            "origin {origin:?}, main {main}, shown {shown_window}, own {own:?}"
                        );
                    }
                }
            }
        }
        // Each route: only a tool window's is kept.
        assert_eq!(dialog_place(Route::Sheet), None);
        assert_eq!(dialog_place(Route::Focus), None);
        assert_eq!(
            dialog_place(Route::Owned(Place::Floating)),
            Some(Place::Floating)
        );
        assert_eq!(
            dialog_place(Route::Owned(place("link"))),
            Some(place("link"))
        );
    }

    /// Whether `p` is a window that exists in the test above: the plugin's
    /// own windows and the origin's window.
    fn own_or_origin(p: &Place, own: &[String], origin: &Option<Origin>) -> bool {
        match p {
            Place::Floating => matches!(origin, Some(Origin::Floating)),
            Place::Options => matches!(origin, Some(Origin::Options)),
            Place::View(id) => own.contains(id),
        }
    }
}
