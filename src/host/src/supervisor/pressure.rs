//! Memory pressure and standby candidate order (systemd-oomd +
//! `OOMScoreAdjust=`; Android LMK importance/LRU). Busy, built-in and
//! resident units are never candidates (A13).
#![forbid(unsafe_code)]

use super::job::Priority;
use super::limits::Tunables;
use super::unit::{Ms, Unit, UnitId, UnitState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pressure {
    #[default]
    None,
    /// Standby jobs for all idle Normal units.
    // B5 reads memory pressure.
    #[cfg_attr(not(test), allow(dead_code))]
    Low,
    /// As `Low`, and no Interactive overrun.
    Critical,
}

/// Whether `unit` may be put in standby by the budget or pressure: an
/// active, idle, reclaimable (Normal) unit with nothing in flight.
pub(super) fn reclaimable<P>(unit: &Unit<P>, tunables: &Tunables) -> bool {
    tunables.policy(unit.class).reclaimable
        && !unit.resident
        && unit.recorded
        && matches!(unit.state, UnitState::Active { .. })
        && unit.job.is_none()
        && !unit.work.busy()
        && unit.backlog.is_empty()
        && !unit.classifying
}

/// `OOMScoreAdjust=` now: the class's, −100 if used recently.
pub(super) fn importance<P>(unit: &Unit<P>, tunables: &Tunables, now: Ms) -> i32 {
    let base = tunables.policy(unit.class).importance;
    let recent = unit
        .last_active()
        .is_some_and(|at| now.saturating_sub(at) < tunables.recent);
    if recent { base - 100 } else { base }
}

/// Candidates in standby order: highest importance first, then least
/// recently used, then unit id (deterministic).
pub(super) fn candidates<'a, P: 'a>(
    units: impl Iterator<Item = (UnitId, &'a Unit<P>)>,
    tunables: &Tunables,
    now: Ms,
) -> Vec<UnitId> {
    let mut list = units
        .filter(|(_, unit)| reclaimable(unit, tunables))
        .map(|(u, unit)| {
            (
                -importance(unit, tunables, now),
                unit.last_active().unwrap_or(0),
                u,
            )
        })
        .collect::<Vec<_>>();
    list.sort();
    list.into_iter().map(|(_, _, u)| u).collect()
}

/// Whether a start of this priority may cause budget standby (event wakes
/// never do, §4.4).
pub(super) fn may_reclaim(priority: Priority) -> bool {
    priority != Priority::Event
}
