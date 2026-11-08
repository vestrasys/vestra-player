//! Tabs on demand in the player's tab columns (doc/architecture/sidebar-tabs.md):
//! the sidebar and, since the menu redesign (step 3b), the adjust column.
//! The tab order, which tabs the shell compiles, which tab keeps its plugin
//! running, and what showing a tab needs: the same rules, per column. Pure
//! decisions; the host applies them (`host/mounts.rs`, `host/window.rs`,
//! `capabilities/ui_mount.rs`).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::ui::composition::Slot;

/// A column of tabs: the sidebar (content, rightmost) or the adjust column
/// (controls for what is playing, between the video and the sidebar). Each
/// has its own tabs, current tab, compiled panels and remembered state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Column {
    Sidebar,
    Adjust,
}

impl Column {
    pub const ALL: [Column; 2] = [Column::Sidebar, Column::Adjust];
    /// Its index in per-column arrays.
    pub fn index(self) -> usize {
        self as usize
    }
    /// The mount slot it shows.
    pub fn slot(self) -> Slot {
        match self {
            Self::Sidebar => Slot::Sidebar,
            Self::Adjust => Slot::Adjust,
        }
    }
    /// The column a mount slot is, if it is one.
    pub fn of(slot: Slot) -> Option<Self> {
        match slot {
            Slot::Sidebar => Some(Self::Sidebar),
            Slot::Adjust => Some(Self::Adjust),
            Slot::Bottom | Slot::Backdrop => None,
        }
    }
    /// The prefix of its shell properties and callbacks (`sidebar-active`,
    /// `adjust-active`, ...).
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::Adjust => "adjust",
        }
    }
    /// Its built-in tabs, in their fixed order before every other tab: the
    /// playlist in the sidebar; Picture, Audio, Subtitles and Playback in the
    /// adjust column (menu redesign §3.3). Other built-ins follow them.
    pub fn first_tabs(self) -> &'static [&'static str] {
        match self {
            Self::Sidebar => &["vestra.playlist"],
            Self::Adjust => &[
                "vestra.video",
                "vestra.audio",
                "vestra.subtitles",
                "vestra.playback",
            ],
        }
    }
    /// Whether the strip is drawn over one tab too: the adjust column always
    /// shows its tabs (with the keys under them); the sidebar from two on.
    pub fn always_tabbed(self) -> bool {
        self == Self::Adjust
    }
}

/// The first tabs in the strip whose panels stay compiled, so switching
/// between them is a property change.
pub const FRONT_TABS: usize = 3;
/// At most this many sidebar panels in a shell: the front tabs and the
/// current one.
pub const MAX_COMPILED: usize = FRONT_TABS + 1;

/// One sidebar tab, as the order needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab<'a> {
    pub owner: u64,
    pub manifest: &'a str,
    pub builtin: bool,
}

/// The tab order of a column: its first tabs (`Column::first_tabs`, in
/// their order), the other built-ins, then the user's order (`remembered`,
/// the column's `order` in `ui_state`, which `remember` appends new plugins
/// to), with ties (and plugins not remembered yet) in owner order.
pub fn order(column: Column, tabs: &[Tab<'_>], remembered: &[String]) -> Vec<u64> {
    let first = column.first_tabs();
    let mut ranked = tabs
        .iter()
        .map(|tab| {
            // The first tabs keep their places (also as development loads).
            let (rank, place) = match first.iter().position(|id| *id == tab.manifest) {
                Some(place) => (0, place),
                None if tab.builtin => (1, 0),
                None => (2, 0),
            };
            let position = remembered
                .iter()
                .position(|known| known == tab.manifest)
                .unwrap_or(usize::MAX);
            ((rank, place, position, tab.owner), tab.owner)
        })
        .collect::<Vec<_>>();
    ranked.sort();
    ranked.into_iter().map(|(_, owner)| owner).collect()
}

/// The front tabs: the tabs the strip can show (it shows at most
/// [`MAX_COMPILED`], `sidebar-tabs.slint`): the first [`FRONT_TABS`] in
/// `order`, then the current one when it is not among them, else the next
/// tab. A narrower strip shows a subset of them (its first places and the
/// current tab), so a tab in the strip always has its panel compiled.
pub fn front(order: &[u64], active: Option<u64>) -> Vec<u64> {
    let mut front = order.iter().take(FRONT_TABS).copied().collect::<Vec<_>>();
    match active {
        Some(active) if order.contains(&active) && !front.contains(&active) => {
            front.push(active);
        }
        _ => front.extend(order.get(FRONT_TABS).copied()),
    }
    front
}

/// The sidebar panels the shell compiles, in tab order: the front tabs
/// whose panel can be drawn. A running plugin's can; a placeholder from a
/// recording (`recorded`) cannot, unless the current shell already has it
/// under the same owner (a plugin parked while its tab was compiled keeps
/// it, with its last values). A panel whose compile failed (`failed`) is
/// left out until its plugin mounts it again. Never more than
/// [`MAX_COMPILED`].
pub fn compiled(
    order: &[u64],
    active: Option<u64>,
    recorded: impl Fn(u64) -> bool,
    failed: impl Fn(u64) -> bool,
    compiled_now: &BTreeMap<u64, u64>,
) -> Vec<u64> {
    let front = front(order, active);
    order
        .iter()
        .copied()
        .filter(|owner| front.contains(owner))
        .filter(|&owner| !failed(owner))
        .filter(|&owner| !recorded(owner) || compiled_now.get(&owner) == Some(&owner))
        .take(MAX_COMPILED)
        .collect()
}

/// The adjust column's compiled panels among its front tabs (`front`, from
/// [`compiled`]): the current tab and the ones the shell already has, so a
/// tab is compiled the first time it is shown (its first opening costs one
/// panel) and then stays, as the front tabs do; switching to a front tab not
/// shown yet compiles it then (`Need::Compile`). Never more than `front`.
pub fn shown_or_compiled(
    front: Vec<u64>,
    active: Option<u64>,
    compiled_now: &BTreeMap<u64, u64>,
) -> Vec<u64> {
    front
        .into_iter()
        .filter(|&owner| Some(owner) == active || compiled_now.contains_key(&owner))
        .collect()
}

/// When a compile of `compiled` failed: the sidebar panels it added without
/// a mount of their own in the `batch` (a switch to their tab, a tab moving
/// into the front). They are left out and reported to their plugin, so a
/// broken panel never fails another plugin's mount; empty when the failure
/// is the batch's own (answered to its plugins, as before).
pub fn blamed(compiled: &[u64], batch: &[u64], compiled_now: &BTreeMap<u64, u64>) -> Vec<u64> {
    compiled
        .iter()
        .copied()
        .filter(|owner| !batch.contains(owner) && compiled_now.get(owner) != Some(owner))
        .collect()
}

/// The current tab after a mount change: `planned`, the policy's choice
/// once the leaving tabs are gone (the next tab takes over a removed
/// current one), unless the current tab left and that choice is a
/// placeholder (`recorded`): then the first running plugin's tab, so that
/// closing a tab never wakes a neighbour unasked.
pub fn next_active(
    order: &[u64],
    planned: Option<u64>,
    current_left: bool,
    recorded: impl Fn(u64) -> bool,
) -> Option<u64> {
    let planned = planned.filter(|owner| order.contains(owner));
    match planned {
        Some(owner) if current_left && recorded(owner) => order
            .iter()
            .copied()
            .find(|&o| !recorded(o))
            .or(Some(owner)),
        planned => planned,
    }
}

/// The branch the shell shows for the current tab (`sidebar-shown`): the
/// owner its panel was compiled under, -1 for a tab without a compiled panel.
pub fn shown(active: Option<u64>, compiled_now: &BTreeMap<u64, u64>) -> i64 {
    active
        .and_then(|owner| compiled_now.get(&owner))
        .map_or(-1, |&alias| alias as i64)
}

/// Whether a panel is something live (supervisor.md §2): the bottom bar and
/// the backdrop are, and make their plugin Resident. A tab of a column (the
/// sidebar, the adjust column) never is (user ruling 2026-11-03): its
/// `ui.mount` is recorded, so its plugin is Normal, and the tab stays as a
/// placeholder while it stands by.
pub fn live(slot: Slot) -> bool {
    Column::of(slot).is_none()
}

/// Whether mount changes still waiting for a plugin (`Some(slot)` a mount,
/// `None` an unmount) make it Resident at its classification: only a
/// waiting bottom bar or backdrop does. A woken run is classified after
/// its placeholder's prune (an unmount) is queued, and a plugin that mounts
/// late or speaks the raw protocol may have its sidebar mount still queued:
/// neither is live.
pub fn pending_live(pending: impl IntoIterator<Item = Option<Slot>>) -> bool {
    pending.into_iter().flatten().any(live)
}

/// Whether `owner`'s panel is the current tab of its column (`active`: that
/// column's current tab): the host's declared work (`Busy::ShownTab`) keeps
/// its plugin running, never parked, while the tab is current; once another
/// tab is, it idles like any Normal plugin.
pub fn shown_tab(slot: Slot, owner: u64, active: Option<u64>) -> bool {
    Column::of(slot).is_some() && active == Some(owner)
}

/// The command whose key a tab shows under its title (the adjust column):
/// the one its plugin's commands name as their `home`
/// (`CommandOptions.home`, the command that opens the panel they live in),
/// the most named one, the first named on a tie. `homes`: each command's
/// `home`, in declaration order.
pub fn tab_command<'a>(homes: impl IntoIterator<Item = Option<&'a str>>) -> Option<&'a str> {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for home in homes.into_iter().flatten() {
        match counts.iter_mut().find(|(id, _)| *id == home) {
            Some((_, count)) => *count += 1,
            None => counts.push((home, 1)),
        }
    }
    let most = counts.iter().map(|(_, count)| *count).max()?;
    counts
        .into_iter()
        .find(|(_, count)| *count == most)
        .map(|(id, _)| id)
}

/// The `ui.mount` call a recording keeps for a background tab: what the
/// plugin sent, without the initial property values (a placeholder draws
/// nothing; the woken plugin mounts again with its own), plus the plugin's
/// name for a tab without a title.
pub fn recorded_call(params: &Value, name: &str) -> Value {
    let mut call = json!({});
    for key in ["slot", "file", "component", "global", "title"] {
        if let Some(value) = params.get(key) {
            call[key] = value.clone();
        }
    }
    if !name.is_empty() {
        call["name"] = name.into();
    }
    call
}

/// What the current tab needs now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// Its panel is in the shell (or comes with a queued mount).
    Nothing,
    /// Its plugin runs and its panel is not compiled: compile the shell.
    Compile,
    /// Its plugin stands by: wake it (its activation mounts the panel).
    Wake,
}

/// What showing tab `owner` needs: `recorded` (a placeholder, no process
/// holds it), `compiled` (its panel is in the shell under this owner),
/// `pending` (a mount of it is queued).
pub fn need(recorded: bool, compiled: bool, pending: bool) -> Need {
    if recorded {
        Need::Wake
    } else if compiled || pending {
        Need::Nothing
    } else {
        Need::Compile
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::ui_state::{AdjustSlot, SidebarSlot};

    fn tabs(ids: &[&'static str]) -> Vec<Tab<'static>> {
        ids.iter()
            .enumerate()
            .map(|(index, id)| Tab {
                owner: 100 + index as u64,
                manifest: id,
                builtin: id.starts_with("vestra."),
            })
            .collect()
    }

    #[test]
    fn built_ins_come_first_then_the_remembered_order() {
        let tabs = tabs(&[
            "c.three",
            "vestra.info",
            "a.one",
            "vestra.playlist",
            "b.two",
        ]);
        let mut slot = SidebarSlot::default();
        let sidebar = |slot: &SidebarSlot| order(Column::Sidebar, &tabs, &slot.order);
        assert_eq!(sidebar(&slot), [103, 101, 100, 102, 104]);
        // The user's order (persisted in ui_state) decides among the others.
        for id in ["b.two", "a.one", "c.three"] {
            slot.remember(id);
        }
        assert_eq!(sidebar(&slot), [103, 101, 104, 102, 100]);
        // Saved and read back, the same order.
        let text = serde_json::to_string(&slot).unwrap();
        let back: SidebarSlot = serde_json::from_str(&text).unwrap();
        assert_eq!(sidebar(&back), [103, 101, 104, 102, 100]);
    }

    /// Menu redesign §3.3: the adjust column's built-in panels come first in
    /// their fixed order (Picture, Audio, Subtitles, Playback), then other
    /// built-ins, then the user's order, new ones last.
    #[test]
    fn the_adjust_column_puts_its_four_built_ins_first_in_their_order() {
        let tabs = tabs(&[
            "z.wheels",
            "vestra.playback",
            "vestra.info",
            "vestra.audio",
            "a.eq",
            "vestra.video",
            "vestra.subtitles",
        ]);
        let mut slot = AdjustSlot::default();
        assert_eq!(
            order(Column::Adjust, &tabs, &slot.order),
            [105, 103, 106, 101, 102, 100, 104]
        );
        slot.remember_among("a.eq", &[]);
        slot.remember_among("z.wheels", &[]);
        assert_eq!(order(Column::Adjust, &tabs, &slot.order)[5..], [104, 100]);
        // The playlist is no first tab there: an ordinary built-in.
        let two = [
            Tab {
                owner: 1,
                manifest: "vestra.video",
                builtin: true,
            },
            Tab {
                owner: 2,
                manifest: "vestra.playlist",
                builtin: true,
            },
        ];
        assert_eq!(order(Column::Adjust, &two, &[]), [1, 2]);
        assert_eq!(order(Column::Sidebar, &two, &[]), [2, 1]);
        assert!(Column::Adjust.always_tabbed() && !Column::Sidebar.always_tabbed());
        for column in Column::ALL {
            assert_eq!(Column::of(column.slot()), Some(column));
        }
        assert_eq!(Column::of(Slot::Bottom), None);
    }

    /// Review of step 3b: the adjust column's first opening compiles the
    /// current tab only; a tab joins when it is shown and then stays.
    #[test]
    fn the_adjust_column_compiles_a_tab_when_it_is_first_shown() {
        let order = [1, 2, 3, 4, 5];
        let mut now = BTreeMap::new();
        let front = |active, now: &BTreeMap<u64, u64>| {
            shown_or_compiled(
                compiled(&order, active, |_| false, |_| false, now),
                active,
                now,
            )
        };
        assert_eq!(front(Some(2), &now), [2]);
        now.insert(2, 2);
        assert_eq!(front(Some(5), &now), [2, 5]);
        now.insert(5, 5);
        // Back to tab 2: tab 5 is no front tab any more, tab 2 stays.
        assert_eq!(front(Some(2), &now), [2]);
        assert_eq!(front(None, &BTreeMap::new()), Vec::<u64>::new());
    }

    #[test]
    fn the_tab_key_is_the_command_the_panels_commands_name_as_home() {
        assert_eq!(tab_command([None, None]), None);
        assert_eq!(
            tab_command([Some("picture"), None, Some("picture")]),
            Some("picture")
        );
        // The most named; the first named on a tie.
        assert_eq!(tab_command([Some("a"), Some("b"), Some("b")]), Some("b"));
        assert_eq!(tab_command([Some("a"), Some("b")]), Some("a"));
    }

    #[test]
    fn twenty_sidebar_plugins_compile_at_most_four_panels() {
        let order = (1..=20).collect::<Vec<u64>>();
        let none = BTreeMap::new();
        for active in [None, Some(1), Some(3), Some(4), Some(20)] {
            let compiled = compiled(&order, active, |_| false, |_| false, &none);
            assert_eq!(compiled.len(), MAX_COMPILED, "{active:?}: {compiled:?}");
            assert_eq!(compiled[..3], [1, 2, 3]);
            if let Some(active) = active {
                assert!(compiled.contains(&active));
            }
        }
        // The strip's four places: the first three and the current tab, or
        // the fourth while the current one is among the first three.
        assert_eq!(
            compiled(&order, Some(17), |_| false, |_| false, &none),
            [1, 2, 3, 17]
        );
        assert_eq!(
            compiled(&order, Some(2), |_| false, |_| false, &none),
            [1, 2, 3, 4]
        );
        assert_eq!(front(&[1, 2], Some(2)), [1, 2]);
    }

    #[test]
    fn no_sidebar_tab_is_live_and_only_the_current_one_is_shown_work() {
        let active = Some(1);
        // Twenty tabs: none makes its plugin Resident (all are recorded);
        // the current one's plugin is kept running by declared work.
        assert!(!live(Slot::Sidebar) && !live(Slot::Adjust));
        assert!(shown_tab(Slot::Adjust, 1, active));
        let shown = (1..=20)
            .filter(|&owner| shown_tab(Slot::Sidebar, owner, active))
            .count();
        assert_eq!(shown, 1);
        assert!(live(Slot::Bottom) && live(Slot::Backdrop));
        assert!(!shown_tab(Slot::Bottom, 1, active));
    }

    #[test]
    fn only_a_waiting_bottom_bar_or_backdrop_mount_is_live() {
        // The prune of a woken run's placeholder, a late sidebar mount.
        assert!(!pending_live([None]));
        assert!(!pending_live([Some(Slot::Sidebar)]));
        assert!(!pending_live([None, Some(Slot::Sidebar)]));
        assert!(!pending_live(std::iter::empty()));
        assert!(pending_live([Some(Slot::Bottom)]));
        assert!(pending_live([None, Some(Slot::Backdrop)]));
    }

    #[test]
    fn placeholders_are_compiled_only_while_the_shell_has_them() {
        let order = [1, 2, 3, 4];
        let mut now = BTreeMap::new();
        let no = |_| false;
        // A placeholder from a recording is never compiled.
        assert_eq!(compiled(&order, Some(1), |o| o == 2, no, &now), [1, 3, 4]);
        // Parked while compiled (same owner): it stays, with its last values.
        now.insert(2, 2);
        assert_eq!(
            compiled(&order, Some(1), |o| o == 2, no, &now),
            [1, 2, 3, 4]
        );
        // Woken under a new owner (rekeyed, the old branch's alias): not
        // drawable as it is; its new mount compiles it.
        now.clear();
        now.insert(2, 9);
        assert_eq!(compiled(&order, Some(1), |o| o == 2, no, &now), [1, 3, 4]);
        assert_eq!(shown(Some(2), &now), 9);
        assert_eq!(shown(Some(4), &now), -1);
        // A panel whose compile failed is left out.
        assert_eq!(compiled(&order, Some(3), no, |o| o == 3, &now), [1, 2, 4]);
    }

    #[test]
    fn a_broken_panel_that_joins_without_a_mount_is_left_out_not_the_batch() {
        let mut now = BTreeMap::new();
        now.insert(1, 1);
        now.insert(2, 2);
        // A switch to tab 7 (no batch): tab 7 broke the compile.
        assert_eq!(blamed(&[1, 2, 7], &[], &now), [7]);
        // A plugin's own new mount that fails is its batch's failure.
        assert!(blamed(&[1, 2, 7], &[7], &now).is_empty());
        // Another plugin's install brings tab 7 in: still tab 7's fault.
        assert_eq!(blamed(&[1, 2, 7], &[9], &now), [7]);
        // Once left out, the set compiles without it.
        let order = [1, 2, 3, 7];
        assert_eq!(
            compiled(&order, Some(7), |_| false, |o| o == 7, &now),
            [1, 2, 3]
        );
    }

    #[test]
    fn closing_the_current_tab_passes_over_a_placeholder_neighbour() {
        let order = [1, 2, 3, 4];
        let recorded = |o| o == 3;
        // The policy hands over to tab 3, a plugin in standby: tab 1 instead.
        assert_eq!(next_active(&order, Some(3), true, recorded), Some(1));
        // A running neighbour is kept.
        assert_eq!(next_active(&order, Some(4), true, recorded), Some(4));
        // The user's choice of a placeholder (not a removal) stands.
        assert_eq!(next_active(&order, Some(3), false, recorded), Some(3));
        // Only placeholders left: the policy's choice.
        assert_eq!(next_active(&[3], Some(3), true, recorded), Some(3));
    }

    #[test]
    fn showing_a_placeholder_wakes_its_plugin_and_a_running_one_compiles() {
        assert_eq!(need(true, false, false), Need::Wake);
        assert_eq!(need(true, true, false), Need::Wake);
        assert_eq!(need(false, false, false), Need::Compile);
        assert_eq!(need(false, true, false), Need::Nothing);
        assert_eq!(need(false, false, true), Need::Nothing);
        // A switch to the 10th of 20 tabs: its panel joins the front tabs.
        let order = (1..=20).collect::<Vec<u64>>();
        assert_eq!(front(&order, Some(10)), [1, 2, 3, 10]);
    }

    #[test]
    fn a_recorded_mount_keeps_what_the_placeholder_needs() {
        let params = json!({"slot":"sidebar","file":"ui/list.slint","component":"List",
            "global":"Bridge","title":"List","properties":{"rows":[1,2,3]}});
        assert_eq!(
            recorded_call(&params, "Example"),
            json!({"slot":"sidebar","file":"ui/list.slint","component":"List",
                "global":"Bridge","title":"List","name":"Example"})
        );
    }
}
