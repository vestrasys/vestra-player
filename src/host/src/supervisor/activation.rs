//! First-activation barrier of one run (systemd `Type=notify` readiness):
//! runtime readiness and every registered initial UI request must finish
//! before success; terminal outcomes never reopen. Callers supply monotonic
//! milliseconds.
//!
//! MOVE (B2a, doc/architecture/supervisor.md §2.1 `activation.rs`) of
//! `host/package_activation.rs`: an `Activation` is the old `Book` entry,
//! now held by its `Unit` (one per unit, for the run being activated)
//! instead of a global map keyed by owner. The Supervisor counts them
//! against `LIMIT` (removed in B3). The UI requests the Host tracks for an
//! activation (`Host.activation_ui`) moved with it (`Run::ui_replies`).
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use serde::Serialize;

use super::unit::Pid;

pub(super) const MAX_SAFE: u64 = (1_u64 << 53) - 1;
/// Activations in flight at once, built-ins included (§0 D1; removed in
/// B3), and initial UI requests per activation.
pub(super) const LIMIT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Status {
    Unknown,
    Pending {
        runtime_ready: bool,
        pending_ui: usize,
    },
    Ready,
    Failed {
        message: String,
    },
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Conflict,
    Full,
    Unknown,
    Terminal,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "invalid activation identity, time or failure message",
            Self::Conflict => "activation or UI registration conflicts with existing state",
            Self::Full => "activation tracking limit reached",
            Self::Unknown => "activation owner or pending UI registration is unknown",
            Self::Terminal => "activation has already reached a terminal outcome",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Debug)]
enum Phase {
    Pending {
        runtime_ready: bool,
        pending_ui: BTreeSet<u64>,
    },
    Ready,
    Failed(String),
    TimedOut,
}

/// One run's activation (the old `Book` entry).
#[derive(Debug)]
pub(super) struct Activation {
    deadline_ms: u64,
    last_now: u64,
    phase: Phase,
}

impl Activation {
    fn new(deadline_ms: u64) -> Self {
        Self {
            deadline_ms,
            last_now: 0,
            phase: Phase::Pending {
                runtime_ready: false,
                pending_ui: BTreeSet::new(),
            },
        }
    }

    /// Validation does not mutate. A valid deadline observation permanently
    /// expires pending activation, even when its requested operation is late.
    pub(super) fn observe(&mut self, now_ms: u64) -> Result<(), Error> {
        if now_ms > MAX_SAFE || now_ms < self.last_now {
            return Err(Error::Invalid);
        }
        self.last_now = now_ms;
        if matches!(self.phase, Phase::Pending { .. }) && now_ms >= self.deadline_ms {
            self.phase = Phase::TimedOut;
        }
        Ok(())
    }

    fn complete_if_ready(&mut self) {
        if matches!(&self.phase, Phase::Pending { runtime_ready: true, pending_ui } if pending_ui.is_empty())
        {
            self.phase = Phase::Ready;
        }
    }

    pub(super) fn status(&self) -> Status {
        match &self.phase {
            Phase::Pending {
                runtime_ready,
                pending_ui,
            } => Status::Pending {
                runtime_ready: *runtime_ready,
                pending_ui: pending_ui.len(),
            },
            Phase::Ready => Status::Ready,
            Phase::Failed(message) => Status::Failed {
                message: message.clone(),
            },
            Phase::TimedOut => Status::TimedOut,
        }
    }

    /// Called after `observe`.
    pub(super) fn ui_begin(&mut self, registration: u64) -> Result<(), Error> {
        match &mut self.phase {
            Phase::Ready => Err(Error::Conflict),
            Phase::Pending { pending_ui, .. } => {
                if pending_ui.contains(&registration) {
                    return Err(Error::Conflict);
                }
                if pending_ui.len() == LIMIT {
                    return Err(Error::Full);
                }
                pending_ui.insert(registration);
                Ok(())
            }
            _ => Err(Error::Terminal),
        }
    }

    /// Called after `observe`.
    pub(super) fn ui_finish(
        &mut self,
        registration: u64,
        result: Result<(), String>,
    ) -> Result<(), Error> {
        let Phase::Pending { pending_ui, .. } = &mut self.phase else {
            return Err(Error::Terminal);
        };
        if !pending_ui.remove(&registration) {
            return Err(Error::Unknown);
        }
        match result {
            Ok(()) => self.complete_if_ready(),
            Err(failure) => self.phase = Phase::Failed(failure),
        }
        Ok(())
    }

    /// Called after `observe`.
    pub(super) fn runtime_ready(&mut self) -> Result<(), Error> {
        let Phase::Pending { runtime_ready, .. } = &mut self.phase else {
            return Err(Error::Terminal);
        };
        *runtime_ready = true;
        self.complete_if_ready();
        Ok(())
    }

    /// Called after `observe`.
    pub(super) fn runtime_failed(&mut self, failure: String) -> Result<(), Error> {
        if !matches!(self.phase, Phase::Pending { .. }) {
            return Err(Error::Terminal);
        }
        self.phase = Phase::Failed(failure);
        Ok(())
    }
}

/// The activation a unit holds: the run (`pid`) it belongs to and the
/// initial UI requests the Host waits on for it (`ui_replies`, the old
/// `Host.activation_ui` entries of that owner: reply ids).
#[derive(Debug)]
pub(super) struct Run {
    pub(super) pid: Pid,
    pub(super) activation: Activation,
    pub(super) ui_replies: BTreeSet<u64>,
}
impl Run {
    pub(super) fn new(pid: Pid, deadline_ms: u64) -> Self {
        Self {
            pid,
            activation: Activation::new(deadline_ms),
            ui_replies: BTreeSet::new(),
        }
    }
}

pub(super) fn identity(value: u64) -> Result<(), Error> {
    if (1..=MAX_SAFE).contains(&value) {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}

pub(super) fn message(value: &str) -> Result<(), Error> {
    if value.len() <= 2048 && !value.contains('\0') {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}

#[cfg(test)]
mod tests {
    //! The old `Book` tests, against the Supervisor: one unit per owner
    //! (as the Host's choke points give each run its unit).
    use super::super::{Supervisor, Tunables, UnitId};
    use super::*;

    struct Book {
        sup: Supervisor<u32>,
    }
    impl Default for Book {
        fn default() -> Self {
            Self {
                sup: Supervisor::new(Tunables::legacy(), 1),
            }
        }
    }
    impl Book {
        fn unit(&mut self, owner: u64) -> UnitId {
            self.sup.load_unit(&format!("a.owner-{owner}"))
        }
        fn begin(&mut self, owner: u64, deadline_ms: u64) -> Result<(), Error> {
            let u = self.unit(owner);
            self.sup.activation_begin(u, Pid(owner), deadline_ms)
        }
        fn ui_begin(&mut self, owner: u64, registration: u64, now_ms: u64) -> Result<(), Error> {
            self.sup
                .activation_ui_begin(Pid(owner), registration, now_ms)
        }
        fn ui_finish(
            &mut self,
            owner: u64,
            registration: u64,
            result: Result<(), String>,
            now_ms: u64,
        ) -> Result<(), Error> {
            self.sup
                .activation_ui_finish(Pid(owner), registration, result, now_ms)
        }
        fn runtime_ready(&mut self, owner: u64, now_ms: u64) -> Result<(), Error> {
            self.sup.activation_ready(Pid(owner), now_ms)
        }
        fn runtime_failed(
            &mut self,
            owner: u64,
            failure: String,
            now_ms: u64,
        ) -> Result<(), Error> {
            self.sup.activation_failed(Pid(owner), failure, now_ms)
        }
        fn status(&mut self, owner: u64, now_ms: u64) -> Result<Status, Error> {
            self.sup.activation_status(Pid(owner), now_ms)
        }
        fn remove(&mut self, owner: u64) {
            self.sup.activation_end(Pid(owner));
        }
    }

    #[test]
    fn runtime_ready_waits_for_all_initial_ui_and_ready_never_reopens() {
        let mut book = Book::default();
        book.begin(1, 100).unwrap();
        book.ui_begin(1, 10, 1).unwrap();
        book.ui_begin(1, 11, 2).unwrap();
        book.runtime_ready(1, 3).unwrap();
        book.ui_finish(1, 10, Ok(()), 4).unwrap();
        assert_eq!(
            book.status(1, 5).unwrap(),
            Status::Pending {
                runtime_ready: true,
                pending_ui: 1
            }
        );
        book.ui_finish(1, 11, Ok(()), 6).unwrap();
        assert_eq!(book.status(1, 100).unwrap(), Status::Ready);
        assert_eq!(book.ui_begin(1, 12, 101), Err(Error::Conflict));
        assert_eq!(
            book.runtime_failed(1, "late".into(), 102),
            Err(Error::Terminal)
        );
        assert_eq!(book.status(1, 103).unwrap(), Status::Ready);
    }

    #[test]
    fn ui_and_runtime_failures_are_sticky() {
        let mut book = Book::default();
        for owner in [1, 2] {
            book.begin(owner, 100).unwrap();
            book.ui_begin(owner, 1, 1).unwrap();
        }
        book.ui_finish(1, 1, Err("UI failed".into()), 2).unwrap();
        book.runtime_failed(2, "runtime failed".into(), 2).unwrap();
        for owner in [1, 2] {
            assert_eq!(book.runtime_ready(owner, 3), Err(Error::Terminal));
            assert_eq!(book.ui_finish(owner, 1, Ok(()), 4), Err(Error::Terminal));
            assert!(matches!(
                book.status(owner, 200).unwrap(),
                Status::Failed { .. }
            ));
        }
    }

    #[test]
    fn deadline_equality_latches_even_without_prior_status_and_owners_are_isolated() {
        let mut book = Book::default();
        book.begin(1, 100).unwrap();
        book.begin(2, 100).unwrap();
        assert_eq!(book.runtime_ready(1, 100), Err(Error::Terminal));
        assert_eq!(book.status(1, 101).unwrap(), Status::TimedOut);
        assert_eq!(book.runtime_ready(1, 102), Err(Error::Terminal));
        assert_eq!(book.status(1, 99), Err(Error::Invalid));
        assert_eq!(
            book.status(2, 99).unwrap(),
            Status::Pending {
                runtime_ready: false,
                pending_ui: 0
            }
        );
        book.runtime_ready(2, 99).unwrap();
        assert_eq!(book.status(2, 101).unwrap(), Status::Ready);
        book.remove(1);
        assert_eq!(book.status(1, 103).unwrap(), Status::Unknown);
        assert_eq!(book.runtime_ready(1, 104), Err(Error::Unknown));
    }

    #[test]
    fn bounds_and_invalid_arguments_do_not_complete_or_fail_activation() {
        let mut book = Book::default();
        assert_eq!(book.begin(0, 1), Err(Error::Invalid));
        assert_eq!(book.begin(1, MAX_SAFE + 1), Err(Error::Invalid));
        for owner in 1..=64 {
            book.begin(owner, 100).unwrap();
        }
        assert_eq!(book.begin(1, 100), Err(Error::Conflict));
        assert_eq!(book.begin(65, 100), Err(Error::Full));
        for registration in 1..=64 {
            book.ui_begin(1, registration, 1).unwrap();
        }
        assert_eq!(book.ui_begin(1, 1, 2), Err(Error::Conflict));
        assert_eq!(book.ui_begin(1, 65, 2), Err(Error::Full));
        assert_eq!(book.ui_begin(1, 0, 3), Err(Error::Invalid));
        assert_eq!(
            book.ui_finish(1, 1, Err("界".repeat(683)), 3),
            Err(Error::Invalid)
        );
        assert_eq!(
            book.runtime_failed(1, "bad\0message".into(), 3),
            Err(Error::Invalid)
        );
        assert_eq!(book.status(1, MAX_SAFE + 1), Err(Error::Invalid));
        assert_eq!(
            book.status(1, 2).unwrap(),
            Status::Pending {
                runtime_ready: false,
                pending_ui: 64
            }
        );
        book.remove(2);
        book.begin(65, 100).unwrap();
    }

    #[test]
    fn status_wire_uses_camel_case_fields_and_unknown_ui_does_not_finish() {
        let mut book = Book::default();
        book.begin(1, 100).unwrap();
        book.ui_begin(1, 7, 1).unwrap();
        assert_eq!(book.ui_finish(1, 8, Ok(()), 2), Err(Error::Unknown));
        let status = book.status(1, 3).unwrap();
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!({"status":"pending","runtimeReady":false,"pendingUi":1})
        );
        assert_eq!(
            book.runtime_failed(9, "missing".into(), 3),
            Err(Error::Unknown)
        );
    }

    /// B0 (doc/architecture/supervisor.md §0 D1, D3): at most 64
    /// activations at once, built-ins included, and an activation leaves
    /// only by `activation_end`: one that timed out or failed keeps its
    /// place.
    #[test]
    fn b0_a_terminal_entry_keeps_its_place_until_removed() {
        let mut book = Book::default();
        for owner in 1..=64 {
            book.begin(owner, 100).unwrap();
        }
        assert_eq!(book.status(1, 100).unwrap(), Status::TimedOut);
        book.runtime_failed(2, "spawn failed".into(), 1).unwrap();
        assert_eq!(book.begin(65, 200), Err(Error::Full));
        book.remove(1);
        book.begin(65, 200).unwrap();
        assert_eq!(book.begin(66, 200), Err(Error::Full));
        book.remove(2);
        book.begin(66, 200).unwrap();
    }

    /// B2a (A18 leak 1): the activation is the unit's, one per unit. A run
    /// whose spawn failed and was never ended cannot hold a place past the
    /// unit's next start: that start's activation replaces it.
    #[test]
    fn a_units_next_activation_replaces_a_stale_one_without_taking_a_place() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let units = (0..64)
            .map(|n| sup.load_unit(&format!("a.fixture-{n:03}")))
            .collect::<Vec<_>>();
        for (n, u) in units.iter().enumerate() {
            let pid = sup.alloc_pid();
            assert_eq!(pid, Pid(n as u64 + 1));
            sup.activation_begin(*u, pid, 100).unwrap();
        }
        // Each unit starts again: the stale activations are replaced, the
        // count stays at 64, and a 65th unit is still refused.
        for u in &units {
            let pid = sup.alloc_pid();
            sup.activation_begin(*u, pid, 200).unwrap();
        }
        assert_eq!(sup.activation_status(Pid(1), 1), Ok(Status::Unknown));
        assert_eq!(sup.by_pid(Pid(1)), None);
        let extra = sup.load_unit("a.fixture-064");
        let pid = sup.alloc_pid();
        assert_eq!(sup.activation_begin(extra, pid, 200), Err(Error::Full));
        sup.activation_end(Pid(65));
        sup.activation_begin(extra, pid, 200).unwrap();
        assert_eq!(sup.by_pid(pid), Some(extra));
    }

    /// The UI requests waited on move with the activation (the old
    /// `Host.activation_ui`): taken once, gone with the activation.
    #[test]
    fn ui_replies_follow_the_activation() {
        let mut sup = Supervisor::<u32>::new(Tunables::legacy(), 1);
        let u = sup.load_unit("a.one");
        let pid = sup.alloc_pid();
        sup.activation_begin(u, pid, 100).unwrap();
        assert!(!sup.activation_ui_waiting(pid));
        sup.activation_ui_wait(pid, 7);
        assert!(sup.activation_ui_waiting(pid));
        assert!(sup.activation_ui_take(pid, 7));
        assert!(!sup.activation_ui_take(pid, 7));
        sup.activation_ui_wait(pid, 8);
        sup.activation_end(pid);
        assert!(!sup.activation_ui_waiting(pid));
        assert!(!sup.activation_ui_take(pid, 8));
        assert_eq!(sup.by_pid(pid), None);
    }
}
