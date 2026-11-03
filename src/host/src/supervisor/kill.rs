//! Stop and kill (systemd `KillSignal=`, `TimeoutStopSec=`,
//! `FinalKillSignal=`): the grace a `Terminate` gets before `Kill`. A stop
//! reaches the unit's own process, or its member in the shared runner (systemd
//! `KillMode=process`); only a runner failure reaches the whole group, as
//! `Action::KillRunner` (`KillMode=control-group`).
#![forbid(unsafe_code)]

use super::limits::Policy;
use super::unit::{Ms, StopReason, Then};

/// `TimeoutStopSec` for a stop: standby 2 s today, after a failure 300 ms,
/// disable/reload/uninstall/upgrade 300 ms today (5 s at B6).
pub fn grace(policy: &Policy, reason: StopReason, then: Then) -> Ms {
    match (reason, then) {
        (StopReason::Failed, _) => policy.failure_grace,
        (_, Then::Standby) => policy.standby_grace,
        _ => policy.stop_grace,
    }
}
