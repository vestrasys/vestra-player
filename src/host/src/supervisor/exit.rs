//! Exit causes (systemd `Result=`, §7.3). Every cause maps onto the wire
//! stop reason `Failed` (A14).
#![forbid(unsafe_code)]

use super::unit::StopReason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitCause {
    /// Exited without being asked (`exit-code`).
    Exited,
    /// Runtime fault, malformed line, uncaught exception (`signal`).
    Fault,
    /// V8 heap abort (`oom-kill`).
    Oom,
    /// `TimeoutStartSec` (`timeout`).
    StartTimeout,
    /// Request timeout (`watchdog`, closest).
    RequestTimeout,
    /// `transport_failed`.
    Transport,
    /// `runtime.failed`: the plugin said it cannot start
    /// (`RestartPreventExitStatus=`): never retried.
    Refused,
}
impl ExitCause {
    /// The wire stop reason (all 7 kept, A14).
    pub fn stop_reason(self) -> StopReason {
        StopReason::Failed
    }
    /// Retried / restarted per policy (all but `Refused`).
    pub fn restartable(self) -> bool {
        self != ExitCause::Refused
    }
    /// A crash notice is shown (T25).
    pub fn notifies(self) -> bool {
        matches!(
            self,
            ExitCause::Exited | ExitCause::Fault | ExitCause::Oom | ExitCause::RequestTimeout
        )
    }
    /// Today's `failures` text where one exists.
    pub fn text(self) -> &'static str {
        match self {
            ExitCause::Exited => "plugin process exited",
            ExitCause::Fault => "plugin runtime fault",
            ExitCause::Oom => "plugin ran out of memory",
            ExitCause::StartTimeout => "activation timed out",
            ExitCause::RequestTimeout => "a request did not finish in time",
            ExitCause::Transport => "plugin transport failed",
            ExitCause::Refused => "plugin reported a failed start",
        }
    }
}
