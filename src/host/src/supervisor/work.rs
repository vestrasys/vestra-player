//! In-flight work per unit (Chrome MV3 "an event in flight extends the
//! lifetime"): the one "busy" (REPLACE the scattered busy checks, B3) and
//! the request deadline (off until B6).
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use super::unit::Ms;

/// What kind of work (the adapter's own numbering: a request id, a
/// predicate such as "settings page open"...).
pub type WorkKind = u64;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Work {
    /// Count and the oldest start of each kind in flight.
    items: BTreeMap<WorkKind, (u32, Ms)>,
}
impl Work {
    // B3: `begin_work`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn begin(&mut self, kind: WorkKind, now: Ms) {
        let entry = self.items.entry(kind).or_insert((0, now));
        entry.0 += 1;
    }
    // B3: `end_work`.
    #[cfg_attr(not(test), allow(dead_code))]
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
    /// Start of the oldest work in flight.
    pub fn oldest(&self) -> Option<Ms> {
        self.items.values().map(|(_, since)| *since).min()
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
        work.begin(1, 10);
        work.begin(1, 20);
        work.begin(2, 5);
        assert_eq!(work.oldest(), Some(5));
        work.end(2);
        work.end(1);
        assert!(work.busy());
        work.end(1);
        assert!(!work.busy());
        work.end(1);
        assert!(!work.busy());
    }
}
