//! Stop and kill: the grace a `Terminate` gets before `Kill`. A stop
//! reaches the unit's own process, or its member in the shared runner; only
//! a runner failure reaches the whole group, as `Action::KillRunner`.
#![forbid(unsafe_code)]

use super::limits::Policy;
use super::unit::{Ms, StopReason, Then};

/// The stop grace (supervisor.md §6): after a failure `failure_grace`, into
/// standby `standby_grace`, otherwise `stop_grace`.
pub fn grace(policy: &Policy, reason: StopReason, then: Then) -> Ms {
    match (reason, then) {
        (StopReason::Failed, _) => policy.failure_grace,
        (_, Then::Standby) => policy.standby_grace,
        _ => policy.stop_grace,
    }
}
