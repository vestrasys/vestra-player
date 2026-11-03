//! In-flight work per unit (Chrome MV3 "an event in flight extends the
//! lifetime"): the one "busy" (B3: the Host's busy checks are work items
//! here, supervisor.md) and the request deadline (off until B6).
//!
//! Two kinds (coordinator ruling): a host→plugin request
//! awaiting the plugin's reply is capped by the class's `request_timeout`
//! (Chrome MV3: 5 min per event); work the plugin or the Host declares
//! (`commands.busy`, a settings page busy, a dialog, a package RPC...) keeps
//! the unit busy without a cap (Android foreground-service precedent).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use super::unit::Ms;

/// One kind of work in flight (the number is the adapter's own: a request
/// id, a predicate such as "settings page open"...).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WorkKind {
    /// A host→plugin request awaiting a reply (`source.resolveRequest`):
    /// capped by `Policy::request_timeout` (B6: 5 min).
    Request(u64),
    /// Declared long work: busy, never capped.
    Declared(u64),
}
impl WorkKind {
    /// Counted against the request deadline.
    pub fn capped(self) -> bool {
        matches!(self, Self::Request(_))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Work {
    /// Count and the oldest start of each kind in flight.
    items: BTreeMap<WorkKind, (u32, Ms)>,
}
impl Work {
    pub fn begin(&mut self, kind: WorkKind, now: Ms) {
        let entry = self.items.entry(kind).or_insert((0, now));
        entry.0 += 1;
    }
    pub fn end(&mut self, kind: WorkKind) {
        if let Some(entry) = self.items.get_mut(&kind) {
            entry.0 -= 1;
            if entry.0 == 0 {
                self.items.remove(&kind);
            }
        }
    }
    pub fn busy(&self) -> bool {
        !self.items.is_empty()
    }
    /// Start of the oldest capped request in flight (declared work has no
    /// deadline).
    pub fn oldest_request(&self) -> Option<Ms> {
        self.items
            .iter()
            .filter(|(kind, _)| kind.capped())
            .map(|(_, (_, since))| *since)
            .min()
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_per_kind() {
        let mut work = Work::default();
        work.begin(WorkKind::Request(1), 10);
        work.begin(WorkKind::Request(1), 20);
        work.begin(WorkKind::Request(2), 5);
        assert_eq!(work.oldest_request(), Some(5));
        work.end(WorkKind::Request(2));
        work.end(WorkKind::Request(1));
        assert!(work.busy());
        work.end(WorkKind::Request(1));
        assert!(!work.busy());
        work.end(WorkKind::Request(1));
        assert!(!work.busy());
    }

    /// Declared work keeps the unit busy but has no request deadline.
    #[test]
    fn declared_work_is_busy_without_a_deadline() {
        let mut work = Work::default();
        work.begin(WorkKind::Declared(7), 1);
        assert!(work.busy());
        assert_eq!(work.oldest_request(), None);
        work.begin(WorkKind::Request(3), 9);
        assert_eq!(work.oldest_request(), Some(9));
        work.end(WorkKind::Request(3));
        assert_eq!(work.oldest_request(), None);
        assert!(work.busy());
    }
}
