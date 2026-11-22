//! Tabs on demand in the player's tab columns (doc/architecture/sidebar-tabs.md):
//! the sidebar and, since the menu redesign (step 3b), the adjust column.
//! The tab order, the current tab after a change, and which tab keeps its
//! plugin running: the same rules, per column (each panel compiles on its
//! own when first shown, `ui/panels.rs`). Pure
//! decisions; the host applies them (`host/mounts.rs`, `host/window.rs`,
//! `api/ui_mount.rs`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};

use crate::ui::composition::Slot;

/// A column of tabs: the sidebar (content, rightmost) or the adjust column
/// (controls for what is playing, between the video and the sidebar). Each
/// has its own tabs, current tab and remembered state.
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

/// The adjust column's front tabs before the current one: see [`front`].
pub const FRONT_TABS: usize = 3;

/// The adjust column's front tabs: its first [`FRONT_TABS`] in `order`,
/// then the current one when it is not among them, else the next tab (the
/// four built-in tabs when nothing else is in front of them). The column
/// compiles all of them whenever it is wanted shown, from its first opening
/// in a run on (user ruling 2026-11-11), so switching among them never
/// compiles (`Host::preload_front`). Never more than
/// `panels::MAX_LOADED`, so none of them is ever unloaded for another.
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

/// During start-up (`settled`: not any more), whether a column waits for
/// its remembered current tab (`remembered`, a plugin id): the policy's
/// current tab (`active`, its plugin id; `None`: none, or not mounted) is
/// another, and no tab of that plugin is among the column's mounted ones
/// (`column`, their plugin ids) yet. Plugins start one after another, and
/// a placeholder replayed from a recording mounts at once: while the
/// column waits, no other tab is current for the host (`Host::current_in`).
pub fn awaits_remembered<'a>(
    settled: bool,
    remembered: Option<&str>,
    active: Option<&str>,
    mut column: impl Iterator<Item = &'a str>,
) -> bool {
    let Some(remembered) = remembered.filter(|_| !settled) else {
        return false;
    };
    active != Some(remembered) && !column.any(|manifest| manifest == remembered)
}

/// Whether a panel is something live (supervisor.md §2): the bottom bar and
/// the backdrop are, and make their plugin Resident. A tab of a column (the
/// sidebar, the adjust column) never is (user ruling 2026-11-03): its
/// `ui.mount` is recorded, so its plugin is Normal, and the tab stays as a
/// placeholder while it stands by.
pub fn live(slot: Slot) -> bool {
    Column::of(slot).is_none()
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

    /// The adjust column's front tabs: the first three and the current one
    /// (else the fourth); never more than a column keeps loaded.
    #[test]
    fn front_tabs_are_the_first_three_and_the_current_one() {
        let order = [1, 2, 3, 4, 5, 6];
        assert_eq!(front(&order, Some(1)), [1, 2, 3, 4]);
        assert_eq!(front(&order, None), [1, 2, 3, 4]);
        assert_eq!(front(&order, Some(4)), [1, 2, 3, 4]);
        assert_eq!(front(&order, Some(6)), [1, 2, 3, 6]);
        // A current tab no longer in the order is not kept.
        assert_eq!(front(&order, Some(9)), [1, 2, 3, 4]);
        assert_eq!(front(&[1, 2], Some(2)), [1, 2]);
        assert_eq!(front(&[], None), Vec::<u64>::new());
        const { assert!(FRONT_TABS < crate::ui::panels::MAX_LOADED) };
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

    /// V2 finding (SDK 1.9.0): at start-up a placeholder replayed from a
    /// recording mounts before the remembered tab's plugin has started, and
    /// became current and woken. Until the remembered tab mounts (or start-up
    /// settles) the column waits: no other tab, placeholder or running, is
    /// current for the host.
    #[test]
    fn a_column_waits_for_its_remembered_tab_until_start_up_settles() {
        let waits = |settled, remembered, active, column: &[&str]| {
            awaits_remembered(settled, remembered, active, column.iter().copied())
        };
        let remembered = Some("vestra.playlist");
        // The replayed placeholder mounted first and is the policy's current tab.
        assert!(waits(
            false,
            remembered,
            Some("zysilm.clipexport"),
            &["zysilm.clipexport"]
        ));
        // Two placeholders, or a running plugin's tab: still waiting.
        assert!(waits(
            false,
            remembered,
            Some("zysilm.clipexport"),
            &["zysilm.clipexport", "zysilm.vestratube"]
        ));
        assert!(waits(false, remembered, None, &[]));
        // The remembered tab mounted: current or not, the wait is over.
        assert!(!waits(
            false,
            remembered,
            Some("vestra.playlist"),
            &["zysilm.clipexport", "vestra.playlist"]
        ));
        assert!(!waits(
            false,
            remembered,
            Some("zysilm.clipexport"),
            &["zysilm.clipexport", "vestra.playlist"]
        ));
        // Start-up settled (the remembered plugin may be gone): no wait.
        assert!(!waits(
            true,
            remembered,
            Some("zysilm.clipexport"),
            &["zysilm.clipexport"]
        ));
        // Nothing remembered (a first start): no wait.
        assert!(!waits(
            false,
            None,
            Some("zysilm.clipexport"),
            &["zysilm.clipexport"]
        ));
        // The same rule in the adjust column.
        assert!(waits(
            false,
            Some("vestra.video"),
            Some("aa.wheels"),
            &["aa.wheels"]
        ));
        assert!(!waits(
            false,
            Some("vestra.video"),
            Some("aa.wheels"),
            &["aa.wheels", "vestra.video"]
        ));
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
