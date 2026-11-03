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

/// The front tabs: the first [`FRONT_TABS`] in `order`, then the current
/// one when it is not among them.
pub fn front(order: &[u64], active: Option<u64>) -> Vec<u64> {
    let mut front = order.iter().take(FRONT_TABS).copied().collect::<Vec<_>>();
    if let Some(active) = active
        && order.contains(&active)
        && !front.contains(&active)
    {
        front.push(active);
    }
    front
}

/// The sidebar panels the shell compiles, in tab order: the front tabs
/// whose panel can be drawn. A running plugin's can; a placeholder from a
/// recording (`recorded`) cannot, unless the current shell already has it
/// under the same owner (a plugin parked while its tab was compiled keeps
/// it, with its last values). Never more than [`MAX_COMPILED`].
pub fn compiled(
    order: &[u64],
    active: Option<u64>,
    recorded: impl Fn(u64) -> bool,
    compiled_now: &BTreeMap<u64, u64>,
) -> Vec<u64> {
    let front = front(order, active);
    order
        .iter()
        .copied()
        .filter(|owner| front.contains(owner))
        .filter(|&owner| !recorded(owner) || compiled_now.get(&owner) == Some(&owner))
        .take(MAX_COMPILED)
        .collect()
}

/// The branch the shell shows for the current tab (`sidebar-shown`): the
/// owner its panel was compiled under, -1 for a tab without a compiled panel.
pub fn shown(active: Option<u64>, compiled_now: &BTreeMap<u64, u64>) -> i64 {
    active
        .and_then(|owner| compiled_now.get(&owner))
        .map_or(-1, |&alias| alias as i64)
}

/// Whether a panel keeps its plugin running (supervisor.md §2, user ruling
/// 2026-11-03): the bottom bar, the backdrop and the current sidebar tab do;
/// a background sidebar tab does not (its plugin may stand by, and its tab
/// stays from its recording).
pub fn live(slot: Slot, owner: u64, active: Option<u64>) -> bool {
    slot != Slot::Sidebar || active == Some(owner)
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
            let compiled = compiled(&order, active, |_| false, &none);
            assert!(compiled.len() <= MAX_COMPILED, "{active:?}: {compiled:?}");
            assert_eq!(compiled[..3], [1, 2, 3]);
            if let Some(active) = active {
                assert!(compiled.contains(&active));
            }
        }
        assert_eq!(compiled(&order, Some(17), |_| false, &none), [1, 2, 3, 17]);
    }

    #[test]
    fn only_the_current_tab_keeps_its_plugin_running() {
        let active = Some(1);
        assert!(live(Slot::Sidebar, 1, active));
        // 19 background tabs: their plugins are Normal and stand by.
        let resident = (1..=20)
            .filter(|&owner| live(Slot::Sidebar, owner, active))
            .count();
        assert_eq!(resident, 1);
        assert!(live(Slot::Bottom, 5, active) && live(Slot::Backdrop, 6, active));
    }

    #[test]
    fn placeholders_are_compiled_only_while_the_shell_has_them() {
        let order = [1, 2, 3, 4];
        let mut now = BTreeMap::new();
        // A placeholder from a recording is never compiled.
        assert_eq!(compiled(&order, Some(1), |o| o == 2, &now), [1, 3]);
        // Parked while compiled (same owner): it stays, with its last values.
        now.insert(2, 2);
        assert_eq!(compiled(&order, Some(1), |o| o == 2, &now), [1, 2, 3]);
        // Woken under a new owner (rekeyed, the old branch's alias): not
        // drawable as it is; its new mount compiles it.
        now.clear();
        now.insert(2, 9);
        assert_eq!(compiled(&order, Some(1), |o| o == 2, &now), [1, 3]);
        assert_eq!(shown(Some(2), &now), 9);
        assert_eq!(shown(Some(4), &now), -1);
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
