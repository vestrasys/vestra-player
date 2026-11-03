//! Exit causes (systemd `Result=`). Every cause maps onto the wire stop reason
//! `Failed`.
#![forbid(unsafe_code)]

use super::unit::StopReason;

/// The exit code of V8's heap abort (Node's `abort()`: 134 on Windows,
/// measured in B4 with Node 26; `SIGABRT` as `128 + 6` on Unix, as the
/// adapter reports a signal). Any abort exits so, so only together with
/// V8's heap-out-of-memory line on stderr is it `Oom`.
pub const HEAP_ABORT_STATUS: i32 = 134;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitCause {
    /// Exited without being asked (`exit-code`).
    Exited,
    /// Runtime fault, malformed line, uncaught exception (`signal`).
    Fault,
    /// V8 heap abort (`oom-kill`): the process exited with
    /// [`HEAP_ABORT_STATUS`] after V8's heap-out-of-memory line on its stderr
    /// (B4).
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
    /// The wire stop reason (all 7 kept).
    pub fn stop_reason(self) -> StopReason {
        StopReason::Failed
    }
    /// Retried / restarted per policy: all but `Refused` and an own process's
    /// `Transport` (`Failed`, KEEP; a member's goes to its runner group, which
    /// restarts; coordinator ruling 2026-11-03).
    pub fn restartable(self) -> bool {
        !matches!(self, ExitCause::Refused | ExitCause::Transport)
    }
    /// A crash notice is shown.
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
