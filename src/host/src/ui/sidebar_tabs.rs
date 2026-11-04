//! Sidebar tabs on demand (doc/architecture/sidebar-tabs.md): the tab
//! order, which tabs the shell compiles, which tab keeps its plugin running,
//! and what showing a tab needs. Pure decisions; the host applies them
//! (`host/mounts.rs`, `host/window.rs`, `capabilities/ui_mount.rs`).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::ui::composition::Slot;
use crate::ui::ui_state::SidebarSlot;

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

/// The tab order: the playlist, the other built-ins, then the user's order
/// (`SidebarSlot::order`, which `remember` appends new plugins to), with
/// ties (and plugins not remembered yet) in owner order.
pub fn order(tabs: &[Tab<'_>], slot: &SidebarSlot) -> Vec<u64> {
    let mut ranked = tabs
        .iter()
        .map(|tab| {
            // The playlist is always first (also as a development load).
            let rank = if tab.manifest == "vestra.playlist" {
                0
            } else if tab.builtin {
                1
            } else {
                2
            };
            ((rank, slot.position(tab.manifest), tab.owner), tab.owner)
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
/// the backdrop are, and make their plugin Resident. A sidebar tab never is
/// (user ruling 2026-11-03): its `ui.mount` is recorded, so its plugin is
/// Normal, and the tab stays as a placeholder while it stands by.
pub fn live(slot: Slot) -> bool {
    slot != Slot::Sidebar
}

/// Whether `owner`'s panel is the current sidebar tab: the host's declared
/// work (`Busy::ShownTab`) keeps its plugin running, never parked, while the
/// tab is current; once another tab is, it idles like any Normal plugin.
pub fn shown_tab(slot: Slot, owner: u64, active: Option<u64>) -> bool {
    slot == Slot::Sidebar && active == Some(owner)
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
        assert_eq!(order(&tabs, &slot), [103, 101, 100, 102, 104]);
        // The user's order (persisted in ui_state) decides among the others.
        for id in ["b.two", "a.one", "c.three"] {
            slot.remember(id);
        }
        assert_eq!(order(&tabs, &slot), [103, 101, 104, 102, 100]);
        // Saved and read back, the same order.
        let text = serde_json::to_string(&slot).unwrap();
        let back: SidebarSlot = serde_json::from_str(&text).unwrap();
        assert_eq!(order(&tabs, &back), [103, 101, 104, 102, 100]);
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
        assert!(!live(Slot::Sidebar));
        let shown = (1..=20)
            .filter(|&owner| shown_tab(Slot::Sidebar, owner, active))
            .count();
        assert_eq!(shown, 1);
        assert!(live(Slot::Bottom) && live(Slot::Backdrop));
        assert!(!shown_tab(Slot::Bottom, 1, active));
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
