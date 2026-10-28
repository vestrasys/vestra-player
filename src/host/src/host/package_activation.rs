//! First-activation barrier, not a complete package installer.
//! Callers supply monotonic milliseconds. Runtime readiness and all registered
//! initial UI work must finish before success; terminal outcomes never reopen.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

const MAX_SAFE: u64 = (1_u64 << 53) - 1;
const LIMIT: usize = 64;

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

enum Phase {
    Pending {
        runtime_ready: bool,
        pending_ui: BTreeSet<u64>,
    },
    Ready,
    Failed(String),
    TimedOut,
}

struct Entry {
    deadline_ms: u64,
    last_now: u64,
    phase: Phase,
}

impl Entry {
    /// Validation does not mutate. A valid deadline observation permanently
    /// expires pending activation, even when its requested operation is late.
    fn observe(&mut self, now_ms: u64) -> Result<(), Error> {
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

    fn status(&self) -> Status {
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
}

#[derive(Default)]
pub struct Book {
    entries: BTreeMap<u64, Entry>,
}

fn identity(value: u64) -> Result<(), Error> {
    if (1..=MAX_SAFE).contains(&value) {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}

fn message(value: &str) -> Result<(), Error> {
    if value.len() <= 2048 && !value.contains('\0') {
        Ok(())
    } else {
        Err(Error::Invalid)
    }
}

impl Book {
    pub fn begin(&mut self, owner: u64, deadline_ms: u64) -> Result<(), Error> {
        identity(owner)?;
        identity(deadline_ms)?;
        if self.entries.contains_key(&owner) {
            return Err(Error::Conflict);
        }
        if self.entries.len() == LIMIT {
            return Err(Error::Full);
        }
        self.entries.insert(
            owner,
            Entry {
                deadline_ms,
                last_now: 0,
                phase: Phase::Pending {
                    runtime_ready: false,
                    pending_ui: BTreeSet::new(),
                },
            },
        );
        Ok(())
    }

    fn entry(&mut self, owner: u64, now_ms: u64) -> Result<&mut Entry, Error> {
        identity(owner)?;
        if now_ms > MAX_SAFE {
            return Err(Error::Invalid);
        }
        let entry = self.entries.get_mut(&owner).ok_or(Error::Unknown)?;
        entry.observe(now_ms)?;
        Ok(entry)
    }

    pub fn ui_begin(&mut self, owner: u64, registration: u64, now_ms: u64) -> Result<(), Error> {
        identity(registration)?;
        let entry = self.entry(owner, now_ms)?;
        match &mut entry.phase {
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

    pub fn ui_finish(
        &mut self,
        owner: u64,
        registration: u64,
        result: Result<(), String>,
        now_ms: u64,
    ) -> Result<(), Error> {
        identity(registration)?;
        if let Err(failure) = &result {
            message(failure)?;
        }
        let entry = self.entry(owner, now_ms)?;
        let Phase::Pending { pending_ui, .. } = &mut entry.phase else {
            return Err(Error::Terminal);
        };
        if !pending_ui.remove(&registration) {
            return Err(Error::Unknown);
        }
        match result {
            Ok(()) => entry.complete_if_ready(),
            Err(failure) => entry.phase = Phase::Failed(failure),
        }
        Ok(())
    }

    pub fn runtime_ready(&mut self, owner: u64, now_ms: u64) -> Result<(), Error> {
        let entry = self.entry(owner, now_ms)?;
        let Phase::Pending { runtime_ready, .. } = &mut entry.phase else {
            return Err(Error::Terminal);
        };
        *runtime_ready = true;
        entry.complete_if_ready();
        Ok(())
    }

    pub fn runtime_failed(
        &mut self,
        owner: u64,
        failure: String,
        now_ms: u64,
    ) -> Result<(), Error> {
        message(&failure)?;
        let entry = self.entry(owner, now_ms)?;
        if !matches!(entry.phase, Phase::Pending { .. }) {
            return Err(Error::Terminal);
        }
        entry.phase = Phase::Failed(failure);
        Ok(())
    }

    pub fn status(&mut self, owner: u64, now_ms: u64) -> Result<Status, Error> {
        identity(owner)?;
        if now_ms > MAX_SAFE {
            return Err(Error::Invalid);
        }
        match self.entries.get_mut(&owner) {
            Some(entry) => {
                entry.observe(now_ms)?;
                Ok(entry.status())
            }
            None => Ok(Status::Unknown),
        }
    }

    pub fn remove(&mut self, owner: u64) {
        self.entries.remove(&owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
