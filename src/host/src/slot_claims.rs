//! Who holds a shell slot, and who claims one first.
//!
//! The shell has one component per slot (`composition::shell_source`). A
//! second plugin asking for a taken slot is refused at `ui.mount` with
//! `conflict` (not later, as a compile failure), and the refusal does not
//! fail its activation (nor does `sequence.set` refused for another
//! provider). While the built-in plugins start, other plugins' mount and
//! sequence requests wait, so a built-in panel (the playlist's sidebar) keeps
//! its slot, and the playlist the sequence, whatever order the processes
//! answer in.
#![forbid(unsafe_code)]

use std::collections::VecDeque;

use crate::composition::Slot;

/// A mount or unmount not yet applied: its owner, and the slot it claims
/// (`None`: an unmount).
pub type Pending = (u64, Option<Slot>);

/// The owner holding `slot` against a claim by `claimant`: a shown mount whose
/// unmount is not already queued, or a queued or preparing mount.
pub fn occupant(
    slot: Slot,
    claimant: u64,
    mounted: impl IntoIterator<Item = (u64, Slot)>,
    pending: &[Pending],
) -> Option<u64> {
    let leaving = |owner: u64| {
        pending
            .iter()
            .any(|&(o, change)| o == owner && change.is_none())
    };
    mounted
        .into_iter()
        .find(|&(owner, s)| s == slot && owner != claimant && !leaving(owner))
        .map(|(owner, _)| owner)
        .or_else(|| {
            pending
                .iter()
                .find(|&&(owner, change)| change == Some(slot) && owner != claimant)
                .map(|&(owner, _)| owner)
        })
}

/// The message of the `conflict` refusal: 「侧栏已被 vestra.playlist 使用」.
pub fn refusal(slot: Slot, holder: &str) -> String {
    format!("{}已被 {holder} 使用", slot.label())
}

/// Mount and sequence requests of non-built-in plugins, held while built-in plugins start.
/// One owner's requests stay in order: once one is held, its later ones are too.
pub struct Gate<T> {
    held: VecDeque<(u64, T)>,
}

impl<T> Default for Gate<T> {
    fn default() -> Self {
        Self {
            held: VecDeque::new(),
        }
    }
}

impl<T> Gate<T> {
    /// Whether a request of `owner` waits: built-ins never do; others while
    /// built-ins start, or behind a request of theirs already waiting.
    pub fn holds(&self, owner: u64, builtin: bool, builtins_starting: bool) -> bool {
        !builtin && (builtins_starting || self.held.iter().any(|(o, _)| *o == owner))
    }

    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    pub fn hold(&mut self, owner: u64, request: T) {
        self.held.push_back((owner, request));
    }

    /// Everything held, oldest first, once built-ins no longer start.
    pub fn release(&mut self, builtins_starting: bool) -> Vec<T> {
        if builtins_starting {
            return Vec::new();
        }
        self.held.drain(..).map(|(_, request)| request).collect()
    }

    /// A stopped plugin's requests are dropped (it is no longer answered).
    pub fn forget(&mut self, owner: u64) {
        self.held.retain(|(o, _)| *o != owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shown_or_queued_mount_holds_its_slot_against_other_owners() {
        let shown = [(1, Slot::Sidebar), (2, Slot::Backdrop)];
        assert_eq!(occupant(Slot::Sidebar, 3, shown, &[]), Some(1));
        assert_eq!(occupant(Slot::Bottom, 3, shown, &[]), None);
        assert_eq!(occupant(Slot::Backdrop, 3, shown, &[]), Some(2));
        // Queued or preparing mounts count, as for the backdrop before.
        assert_eq!(
            occupant(Slot::Bottom, 3, shown, &[(4, Some(Slot::Bottom))]),
            Some(4)
        );
        // The claimant's own entries never conflict.
        assert_eq!(occupant(Slot::Sidebar, 1, shown, &[]), None);
        assert_eq!(
            occupant(Slot::Bottom, 4, [], &[(4, Some(Slot::Bottom))]),
            None
        );
    }

    #[test]
    fn a_queued_unmount_frees_the_slot() {
        let shown = [(1, Slot::Sidebar)];
        assert_eq!(occupant(Slot::Sidebar, 3, shown, &[(1, None)]), None);
        // Another owner's unmount does not.
        assert_eq!(occupant(Slot::Sidebar, 3, shown, &[(2, None)]), Some(1));
        // A mount queued behind the unmount takes the slot.
        assert_eq!(
            occupant(
                Slot::Sidebar,
                3,
                shown,
                &[(1, None), (2, Some(Slot::Sidebar))]
            ),
            Some(2)
        );
    }

    #[test]
    fn refusal_names_the_slot_and_its_holder() {
        assert_eq!(
            refusal(Slot::Sidebar, "vestra.playlist"),
            "侧栏已被 vestra.playlist 使用"
        );
        assert_eq!(refusal(Slot::Bottom, "x.y"), "底栏已被 x.y 使用");
        assert_eq!(refusal(Slot::Backdrop, "x.y"), "视频背景已被 x.y 使用");
    }

    #[test]
    fn non_builtin_mounts_wait_for_starting_builtins_and_keep_their_order() {
        let mut gate = Gate::default();
        // Built-ins never wait; others do only while built-ins start.
        assert!(!gate.holds(1, true, true));
        assert!(!gate.holds(2, false, false));
        assert!(gate.holds(2, false, true));
        gate.hold(2, "mount 2");
        gate.hold(3, "mount 3");
        assert!(gate.release(true).is_empty());
        // Behind its held mount, the owner's unmount waits even after start-up.
        assert!(gate.holds(2, false, false));
        assert!(!gate.holds(4, false, false));
        gate.hold(2, "unmount 2");
        assert_eq!(gate.release(false), ["mount 2", "mount 3", "unmount 2"]);
        assert!(gate.is_empty());
        assert!(!gate.holds(2, false, false));
        assert!(gate.release(false).is_empty());
    }

    #[test]
    fn a_stopped_plugin_loses_its_held_requests() {
        let mut gate = Gate::default();
        gate.hold(2, 20);
        gate.hold(3, 30);
        gate.hold(2, 21);
        gate.forget(2);
        assert_eq!(gate.release(false), [30]);
    }
}
