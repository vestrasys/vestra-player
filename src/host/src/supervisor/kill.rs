//! Stop and kill (systemd `KillMode=`, `KillSignal=`, `TimeoutStopSec=`,
//! `FinalKillSignal=`): the grace a `Terminate` gets before `Kill`.
#![forbid(unsafe_code)]

use super::limits::Policy;
use super::unit::{Ms, Placement, StopReason, Then};

/// What a stop/kill reaches (§3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KillMode {
    /// The unit's own process (or its member in the runner).
    Process,
    /// The whole runner (all members): a runner failure.
    RunnerGroup,
}

/// `KillMode` of a unit's stop.
pub fn kill_mode(_placement: Placement) -> KillMode {
    // A member's stop goes to the runner as a member stop; only a runner
    // failure reaches the group (`runner_failed`).
    KillMode::Process
}

/// `TimeoutStopSec` for a stop (§7.2): standby 2 s today, after a failure
/// 300 ms, disable/reload/uninstall/upgrade 300 ms today (5 s at B6).
pub fn grace(policy: &Policy, reason: StopReason, then: Then) -> Ms {
    match (reason, then) {
        (StopReason::Failed, _) => policy.failure_grace,
        (_, Then::Standby) => policy.standby_grace,
        _ => policy.stop_grace,
    }
}
