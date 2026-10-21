//! M1 source arbitration (host-internal).
//! Backend adapters must correlate events from real playlist entries and ordered
//! StartFile/FileLoaded delivery; never label an event using the polled source.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

pub type OwnerGeneration = u64;
pub type EntryId = i64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStamp {
    pub generation: u64,
    pub intent_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Empty,
    Loading,
    Ready,
    Ended,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EndReason {
    Natural,
    Stopped,
    Replaced,
    Failed(String),
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Control {
    Pause { paused: bool },
    Seek { seconds: f64 },
    Rate { rate: f64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SourceIntent {
    OpenProvider {
        producer_id: u64,
        start_paused: bool,
        expected: Option<SourceStamp>,
    },
    Open {
        source: String,
        start_paused: bool,
        expected: Option<SourceStamp>,
    },
    Restart {
        expected: Option<SourceStamp>,
    },
    Close {
        expected: Option<SourceStamp>,
    },
    Control {
        control: Control,
        expected: Option<SourceStamp>,
    },
}

impl SourceIntent {
    fn expected(&self) -> Option<SourceStamp> {
        match self {
            Self::OpenProvider { expected, .. }
            | Self::Open { expected, .. }
            | Self::Restart { expected }
            | Self::Close { expected }
            | Self::Control { expected, .. } => *expected,
        }
    }
}

/// Identifies the backend load cycle independently of subsequent pause/seek intents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntentTicket {
    pub stamp: SourceStamp,
    pub cycle: u64,
    pub owner: OwnerGeneration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceState {
    pub stamp: SourceStamp,
    pub cycle: u64,
    pub event_sequence: u64,
    pub phase: Phase,
    pub source: Option<String>,
    pub loading_owner: Option<OwnerGeneration>,
    pub terminal_reason: Option<EndReason>,
    pub paused: bool,
    pub position: Option<f64>,
    pub duration: Option<f64>,
}

/// FileLoaded has no native entry ID: the adapter supplies the ticket associated
/// with its preceding real StartFile event. `Ready` must not be built from state().
#[derive(Clone, Debug)]
pub enum SourceEvent {
    #[cfg_attr(not(feature = "producer-audio"), allow(dead_code))]
    ProviderReady {
        ticket: IntentTicket,
    },
    StartFile {
        entry: EntryId,
        ticket: IntentTicket,
    },
    Ready {
        ticket: IntentTicket,
    },
    EndFile {
        entry: EntryId,
        reason: EndReason,
    },
    LoadFailed {
        ticket: IntentTicket,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RequestError {
    Stale,
    Invalid,
    NoSource,
    Conflict,
    Busy,
    UnknownExpired,
    Backend(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum OperationStatus {
    Pending { ticket: IntentTicket },
    Completed { ticket: IntentTicket },
    Failed { error: RequestError },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    Execute(IntentTicket),
    Replay(OperationStatus),
    Rejected(RequestError),
}

struct OperationRecord {
    intent: SourceIntent,
    status: OperationStatus,
}

/// One book per live owner generation. IDs are positive strictly increasing
/// request numbers, generated before transmission. Lower unseen or evicted IDs
/// return UnknownExpired, including resubmission; they NEVER execute again.
/// Out-of-order delivery is intentionally rejected. Dropping the owner drops its
/// book; a replacement instance needs a distinct owner generation and fresh book.
pub struct OperationBook {
    pub owner: OwnerGeneration,
    high_water: u64,
    records: BTreeMap<u64, OperationRecord>,
    terminal: VecDeque<u64>,
    pending: usize,
}

impl OperationBook {
    pub const MAX_PENDING: usize = 128;
    pub const MAX_TERMINAL: usize = 512;

    pub fn new(owner: OwnerGeneration) -> Self {
        Self {
            owner,
            high_water: 0,
            records: BTreeMap::new(),
            terminal: VecDeque::new(),
            pending: 0,
        }
    }

    pub fn query(&self, id: u64) -> Result<OperationStatus, RequestError> {
        self.records
            .get(&id)
            .map(|record| record.status.clone())
            .ok_or(RequestError::UnknownExpired)
    }

    fn existing(&self, id: u64, intent: &SourceIntent) -> Option<Admission> {
        if let Some(record) = self.records.get(&id) {
            return Some(if record.intent == *intent {
                Admission::Replay(record.status.clone())
            } else {
                Admission::Rejected(RequestError::Conflict)
            });
        }
        (id <= self.high_water).then_some(Admission::Rejected(RequestError::UnknownExpired))
    }

    fn insert(&mut self, id: u64, intent: SourceIntent, status: OperationStatus) {
        self.high_water = id;
        let pending = matches!(status, OperationStatus::Pending { .. });
        self.records.insert(id, OperationRecord { intent, status });
        if pending {
            self.pending += 1;
        } else {
            self.retain_terminal(id);
        }
    }

    fn retain_terminal(&mut self, id: u64) {
        self.terminal.push_back(id);
        while self.terminal.len() > Self::MAX_TERMINAL {
            if let Some(expired) = self.terminal.pop_front() {
                self.records.remove(&expired);
            }
        }
    }

    /// Command completion is distinct from FileLoaded/Ready. A completed open
    /// means the backend accepted the load, not that decoding succeeded.
    pub fn finish(
        &mut self,
        id: u64,
        result: Result<(), String>,
    ) -> Result<OperationStatus, RequestError> {
        let record = self
            .records
            .get_mut(&id)
            .ok_or(RequestError::UnknownExpired)?;
        if let OperationStatus::Pending { ticket } = record.status {
            record.status = match result {
                Ok(()) => OperationStatus::Completed { ticket },
                Err(message) => OperationStatus::Failed {
                    error: RequestError::Backend(message),
                },
            };
            self.pending -= 1;
            let status = record.status.clone();
            self.retain_terminal(id);
            Ok(status)
        } else {
            Ok(record.status.clone())
        }
    }
}

pub struct SourceSession {
    state: SourceState,
    entries: BTreeMap<EntryId, IntentTicket>,
    load_ticket: Option<IntentTicket>,
}

impl Default for SourceSession {
    fn default() -> Self {
        Self {
            state: SourceState {
                stamp: SourceStamp::default(),
                cycle: 0,
                event_sequence: 0,
                phase: Phase::Empty,
                source: None,
                loading_owner: None,
                terminal_reason: None,
                paused: false,
                position: None,
                duration: None,
            },
            entries: BTreeMap::new(),
            load_ticket: None,
        }
    }
}

impl SourceSession {
    pub fn state(&self) -> &SourceState {
        &self.state
    }

    /// Call at the same serial arbitration point as backend command dispatch.
    /// No effect/style update belongs here: those compare generation + resource
    /// revision separately and must not advance or constrain source intent.
    #[cfg_attr(not(feature = "producer-audio"), allow(dead_code))]
    pub fn submit(&mut self, book: &mut OperationBook, id: u64, intent: SourceIntent) -> Admission {
        self.submit_checked(book, id, intent, |_| Ok(()))
    }
    /// Adapter preflight runs only for a new request, after retained replay/conflict
    /// checks, before any source facts/intent are changed.
    pub fn submit_checked(
        &mut self,
        book: &mut OperationBook,
        id: u64,
        intent: SourceIntent,
        check: impl FnOnce(&SourceIntent) -> Result<(), RequestError>,
    ) -> Admission {
        if let Some(existing) = book.existing(id, &intent) {
            return existing;
        }
        if book.pending >= OperationBook::MAX_PENDING {
            // Remember refusal too: retrying an unknown result must not execute.
            book.insert(
                id,
                intent,
                OperationStatus::Failed {
                    error: RequestError::Busy,
                },
            );
            return Admission::Rejected(RequestError::Busy);
        }
        if let Err(error) = self.validate(&intent).and_then(|_| check(&intent)) {
            book.insert(
                id,
                intent,
                OperationStatus::Failed {
                    error: error.clone(),
                },
            );
            return Admission::Rejected(error);
        }
        self.state.stamp.intent_version += 1;
        match &intent {
            SourceIntent::OpenProvider {
                producer_id,
                start_paused,
                ..
            } => {
                self.state.stamp.generation += 1;
                self.state.source = Some(format!("producer://{producer_id}"));
                self.state.paused = *start_paused;
                self.begin_cycle(book.owner);
            }
            SourceIntent::Open {
                source,
                start_paused,
                ..
            } => {
                self.state.stamp.generation += 1;
                self.state.source = Some(source.clone());
                self.state.paused = *start_paused;
                self.begin_cycle(book.owner);
            }
            SourceIntent::Restart { .. } => {
                self.state.paused = false;
                self.begin_cycle(book.owner);
            }
            SourceIntent::Close { .. } => {
                self.state.phase = Phase::Empty;
                self.state.source = None;
                self.state.position = None;
                self.state.duration = None;
                self.state.loading_owner = None;
                self.state.terminal_reason = Some(EndReason::Stopped);
                self.entries.clear();
                self.load_ticket = None;
            }
            SourceIntent::Control { .. } => {}
        }
        let ticket = IntentTicket {
            stamp: self.state.stamp,
            cycle: self.state.cycle,
            owner: book.owner,
        };
        if matches!(
            intent,
            SourceIntent::Open { .. }
                | SourceIntent::OpenProvider { .. }
                | SourceIntent::Restart { .. }
        ) {
            self.load_ticket = Some(ticket);
        }
        self.state.event_sequence += 1;
        book.insert(id, intent, OperationStatus::Pending { ticket });
        Admission::Execute(ticket)
    }

    /// Backend-confirmed facts only. The adapter supplies the actual active
    /// entry's ticket, never a ticket manufactured from the current snapshot.
    pub fn observe(
        &mut self,
        ticket: IntentTicket,
        paused: bool,
        position: Option<f64>,
        duration: Option<f64>,
    ) -> bool {
        if !self.current_cycle(ticket) || !matches!(self.state.phase, Phase::Ready | Phase::Ended) {
            return false;
        }
        self.state.paused = paused;
        self.state.position = position.filter(|n| n.is_finite());
        self.state.duration = duration.filter(|n| n.is_finite() && *n >= 0.);
        self.state.event_sequence += 1;
        true
    }

    /// A successful seek away from keep-open EOF starts a fresh playback cycle
    /// on the same native entry. It is not a new load and must be re-armed for
    /// the next EOF. The accepted control ticket names the previous cycle.
    pub fn confirm_seek_from_end(
        &mut self,
        ticket: IntentTicket,
        entry: EntryId,
    ) -> Option<IntentTicket> {
        if self.state.phase != Phase::Ended || ticket.stamp != self.state.stamp {
            return None;
        }
        self.state.cycle += 1;
        self.state.phase = Phase::Ready;
        self.state.terminal_reason = None;
        self.state.event_sequence += 1;
        let active = IntentTicket {
            cycle: self.state.cycle,
            ..ticket
        };
        self.load_ticket = Some(active);
        self.entries.clear();
        self.entries.insert(entry, active);
        Some(active)
    }

    fn validate(&self, intent: &SourceIntent) -> Result<(), RequestError> {
        if intent
            .expected()
            .is_some_and(|expected| expected != self.state.stamp)
        {
            return Err(RequestError::Stale);
        }
        match intent {
            SourceIntent::OpenProvider { producer_id, .. }
                if *producer_id == 0 || *producer_id > 9_007_199_254_740_991 =>
            {
                Err(RequestError::Invalid)
            }
            SourceIntent::Open { source, .. } if source.trim().is_empty() => {
                Err(RequestError::Invalid)
            }
            SourceIntent::Restart { .. } | SourceIntent::Control { .. }
                if self.state.source.is_none() =>
            {
                Err(RequestError::NoSource)
            }
            SourceIntent::Control {
                control: Control::Seek { seconds },
                ..
            } if !seconds.is_finite() || *seconds < 0.0 => Err(RequestError::Invalid),
            SourceIntent::Control {
                control: Control::Rate { rate },
                ..
            } if !rate.is_finite() || !(0.05..=10.0).contains(rate) => Err(RequestError::Invalid),
            _ => Ok(()),
        }
    }

    fn begin_cycle(&mut self, owner: OwnerGeneration) {
        self.state.cycle += 1;
        self.state.phase = Phase::Loading;
        self.state.loading_owner = Some(owner);
        self.state.terminal_reason = None;
        self.state.position = None;
        self.state.duration = None;
        self.entries.clear();
    }

    fn current_cycle(&self, ticket: IntentTicket) -> bool {
        self.load_ticket == Some(ticket)
            && ticket.stamp.generation == self.state.stamp.generation
            && ticket.cycle == self.state.cycle
    }

    /// Returns whether public facts changed. Ignored late events do not advance
    /// eventSequence. Ended retains identity and real reason for auto-next CAS.
    pub fn event(&mut self, event: SourceEvent) -> bool {
        match event {
            SourceEvent::ProviderReady { ticket } => {
                if !self.current_cycle(ticket) || self.state.phase != Phase::Loading {
                    return false;
                }
                self.state.phase = Phase::Ready;
                self.state.loading_owner = None;
            }
            SourceEvent::StartFile { entry, ticket } => {
                if !self.current_cycle(ticket) || self.state.phase != Phase::Loading {
                    return false;
                }
                // A load cycle has one active mpv playlist entry. Replacement
                // StartFile explicitly supersedes the earlier entry association.
                self.entries.clear();
                self.entries.insert(entry, ticket);
                return false;
            }
            SourceEvent::Ready { ticket } => {
                if !self.current_cycle(ticket)
                    || self.state.phase != Phase::Loading
                    || !self
                        .entries
                        .values()
                        .any(|associated| *associated == ticket)
                {
                    return false;
                }
                self.state.phase = Phase::Ready;
                self.state.loading_owner = None;
            }
            SourceEvent::EndFile { entry, reason } => {
                let Some(ticket) = self.entries.remove(&entry) else {
                    return false;
                };
                if !self.current_cycle(ticket)
                    || matches!(self.state.phase, Phase::Empty | Phase::Ended | Phase::Error)
                {
                    return false;
                }
                self.state.phase = if matches!(reason, EndReason::Failed(_)) {
                    Phase::Error
                } else {
                    Phase::Ended
                };
                self.state.terminal_reason = Some(reason);
                self.state.loading_owner = None;
            }
            SourceEvent::LoadFailed { ticket, message } => {
                if !self.current_cycle(ticket)
                    || !matches!(self.state.phase, Phase::Loading | Phase::Ready)
                {
                    return false;
                }
                self.state.phase = Phase::Error;
                self.state.terminal_reason = Some(EndReason::Failed(message));
                self.state.loading_owner = None;
                self.entries.clear();
            }
        }
        self.state.event_sequence += 1;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_preflight_preserves_source_and_retained_operation_priority() {
        let mut session = SourceSession::default();
        let mut book = OperationBook::new(7);
        let first = open(&mut session, &mut book, 1, "file");
        ready(&mut session, first, 1);
        book.finish(1, Ok(())).unwrap();
        let before = serde_json::to_value(session.state()).unwrap();
        let intent = SourceIntent::OpenProvider {
            producer_id: 2,
            start_paused: false,
            expected: Some(session.state().stamp),
        };
        assert!(matches!(
            session.submit_checked(&mut book, 2, intent.clone(), |_| Err(
                RequestError::Backend("not ready".into())
            )),
            Admission::Rejected(_)
        ));
        assert_eq!(serde_json::to_value(session.state()).unwrap(), before);
        assert!(matches!(
            session.submit_checked(&mut book, 2, intent.clone(), |_| panic!(
                "replay must not preflight"
            )),
            Admission::Replay(OperationStatus::Failed { .. })
        ));
        assert!(matches!(
            session.submit_checked(
                &mut book,
                2,
                SourceIntent::Close { expected: None },
                |_| panic!("conflict must not preflight")
            ),
            Admission::Rejected(RequestError::Conflict)
        ));
        let ticket = match session.submit_checked(&mut book, 3, intent, |_| Ok(())) {
            Admission::Execute(t) => t,
            _ => panic!("provider admission"),
        };
        assert!(session.event(SourceEvent::ProviderReady { ticket }));
        assert_eq!(session.state().stamp.generation, 2);
        assert!(!session.event(SourceEvent::Ready { ticket: first }));
    }

    fn open(
        session: &mut SourceSession,
        book: &mut OperationBook,
        id: u64,
        source: &str,
    ) -> IntentTicket {
        match session.submit(
            book,
            id,
            SourceIntent::Open {
                source: source.into(),
                start_paused: false,
                expected: None,
            },
        ) {
            Admission::Execute(ticket) => ticket,
            other => panic!("open failed: {other:?}"),
        }
    }

    fn ready(session: &mut SourceSession, ticket: IntentTicket, entry: EntryId) {
        session.event(SourceEvent::StartFile { entry, ticket });
        assert!(session.event(SourceEvent::Ready { ticket }));
    }

    #[test]
    fn late_a_ready_and_error_cannot_overwrite_b_loading() {
        let mut session = SourceSession::default();
        let mut owner = OperationBook::new(7);
        let a = open(&mut session, &mut owner, 1, "a");
        session.event(SourceEvent::StartFile {
            entry: 10,
            ticket: a,
        });
        let b = open(&mut session, &mut owner, 2, "b");
        let sequence = session.state().event_sequence;
        assert!(!session.event(SourceEvent::Ready { ticket: a }));
        assert!(!session.event(SourceEvent::LoadFailed {
            ticket: a,
            message: "old error".into()
        }));
        assert!(!session.event(SourceEvent::EndFile {
            entry: 10,
            reason: EndReason::Failed("old error".into())
        }));
        assert_eq!(session.state().event_sequence, sequence);
        assert_eq!(session.state().source.as_deref(), Some("b"));
        assert_eq!(session.state().duration, None);
        ready(&mut session, b, 11);
        assert_eq!(session.state().phase, Phase::Ready);
    }

    #[test]
    fn ended_auto_next_loses_to_manual_open_and_restart() {
        for restart in [false, true] {
            let mut session = SourceSession::default();
            let mut owner = OperationBook::new(7);
            let a = open(&mut session, &mut owner, 1, "a");
            ready(&mut session, a, 10);
            assert!(session.event(SourceEvent::EndFile {
                entry: 10,
                reason: EndReason::Natural
            }));
            let ended = session.state().stamp;
            assert_eq!(session.state().terminal_reason, Some(EndReason::Natural));
            let intent = if restart {
                SourceIntent::Restart {
                    expected: Some(ended),
                }
            } else {
                SourceIntent::Open {
                    source: "manual".into(),
                    start_paused: false,
                    expected: Some(ended),
                }
            };
            assert!(matches!(
                session.submit(&mut owner, 2, intent),
                Admission::Execute(_)
            ));
            let stamp = session.state().stamp;
            assert_eq!(stamp.generation, ended.generation + u64::from(!restart));
            assert_eq!(
                session.submit(
                    &mut owner,
                    3,
                    SourceIntent::Open {
                        source: "auto-next".into(),
                        start_paused: false,
                        expected: Some(ended)
                    }
                ),
                Admission::Rejected(RequestError::Stale)
            );
            assert_eq!(session.state().stamp, stamp);
            assert!(!session.event(SourceEvent::EndFile {
                entry: 10,
                reason: EndReason::Natural
            }));
            assert!(!session.event(SourceEvent::Ready { ticket: a }));
            assert_eq!(session.state().phase, Phase::Loading);
        }
    }

    #[test]
    fn controls_invalidate_auto_next_without_breaking_real_load_association() {
        let mut session = SourceSession::default();
        let mut owner = OperationBook::new(1);
        let a = open(&mut session, &mut owner, 1, "a");
        assert!(matches!(
            session.submit(
                &mut owner,
                2,
                SourceIntent::Control {
                    control: Control::Pause { paused: true },
                    expected: Some(a.stamp)
                }
            ),
            Admission::Execute(_)
        ));
        ready(&mut session, a, 5);
        session.event(SourceEvent::EndFile {
            entry: 5,
            reason: EndReason::Natural,
        });
        let sequence = session.state().event_sequence;
        assert!(!session.event(SourceEvent::EndFile {
            entry: 5,
            reason: EndReason::Natural
        }));
        assert_eq!(session.state().event_sequence, sequence);
        assert_ne!(session.state().stamp, a.stamp);
    }

    #[test]
    fn replay_conflict_and_expiration_never_repeat_side_effects() {
        let mut session = SourceSession::default();
        let mut owner = OperationBook::new(9);
        let intent = SourceIntent::Open {
            source: "a".into(),
            start_paused: false,
            expected: None,
        };
        let ticket = open(&mut session, &mut owner, 1, "a");
        assert_eq!(
            session.submit(&mut owner, 1, intent.clone()),
            Admission::Replay(OperationStatus::Pending { ticket })
        );
        assert_eq!(
            session.submit(&mut owner, 1, SourceIntent::Close { expected: None }),
            Admission::Rejected(RequestError::Conflict)
        );
        owner.finish(1, Ok(())).unwrap();
        for id in 2..=514 {
            open(&mut session, &mut owner, id, "new");
            owner.finish(id, Ok(())).unwrap();
        }
        let stamp = session.state().stamp;
        assert_eq!(owner.records.len(), OperationBook::MAX_TERMINAL);
        assert_eq!(owner.query(1), Err(RequestError::UnknownExpired));
        assert_eq!(
            session.submit(&mut owner, 1, intent),
            Admission::Rejected(RequestError::UnknownExpired)
        );
        assert_eq!(session.state().stamp, stamp);
    }

    #[test]
    fn pending_pressure_is_bounded_and_refusal_is_idempotent() {
        let mut session = SourceSession::default();
        let mut owner = OperationBook::new(1);
        for id in 1..=128 {
            open(&mut session, &mut owner, id, "a");
        }
        let intent = SourceIntent::Close { expected: None };
        let stamp = session.state().stamp;
        assert_eq!(
            session.submit(&mut owner, 129, intent.clone()),
            Admission::Rejected(RequestError::Busy)
        );
        owner.finish(1, Ok(())).unwrap();
        assert_eq!(
            session.submit(&mut owner, 129, intent),
            Admission::Replay(OperationStatus::Failed {
                error: RequestError::Busy
            })
        );
        assert_eq!(session.state().stamp, stamp);
        assert_eq!(owner.pending, 127);
    }
}
