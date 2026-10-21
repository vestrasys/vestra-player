//! Bounded input delivery (host-internal, not the plugin API).
//! The adapter supplies a monotonic millisecond clock and stops the source on
//! timeout. This module performs no IO and never silently drops an expired item.
#![forbid(unsafe_code)]

use std::collections::VecDeque;

use crate::producer_input::{self, Notice, RevokeReason, Scope, State};

const MAX_SAFE: u64 = (1_u64 << 53) - 1;
const CAPACITY: usize = 32;
const ACK_TIMEOUT_MS: u64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Stale,
    Unavailable,
    Busy,
    Exhausted,
    TimedOut,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "invalid input delivery time or scope",
            Self::Stale => "input acknowledgement does not match the current notice",
            Self::Unavailable => "input delivery is unavailable",
            Self::Busy => "input delivery awaits acknowledgement or has a full queue",
            Self::Exhausted => "input notice sequence exhausted",
            Self::TimedOut => "input acknowledgement deadline expired",
        })
    }
}

impl std::error::Error for Error {}

impl From<producer_input::Error> for Error {
    fn from(error: producer_input::Error) -> Self {
        match error {
            producer_input::Error::Invalid => Self::Invalid,
            producer_input::Error::Stale => Self::Stale,
            producer_input::Error::Unavailable => Self::Unavailable,
            producer_input::Error::Exhausted => Self::Exhausted,
        }
    }
}

struct Pending {
    notice: Notice,
    deadline_ms: u64,
}

pub struct Delivery {
    state: State,
    queue: VecDeque<Pending>,
    revoke_barrier: bool,
    last_receipt: Option<Notice>,
}

impl Delivery {
    pub fn new(state: State) -> Self {
        Self {
            state,
            queue: VecDeque::new(),
            revoke_barrier: false,
            last_receipt: None,
        }
    }

    fn check_time(&self, now_ms: u64) -> Result<(), Error> {
        if now_ms > MAX_SAFE {
            return Err(Error::Invalid);
        }
        if self.expired(now_ms) {
            return Err(Error::TimedOut);
        }
        Ok(())
    }

    fn deadline(&self, now_ms: u64) -> Result<u64, Error> {
        self.check_time(now_ms)?;
        now_ms
            .checked_add(ACK_TIMEOUT_MS)
            .filter(|deadline| *deadline <= MAX_SAFE)
            .ok_or(Error::Invalid)
    }

    fn admit(&self, now_ms: u64) -> Result<u64, Error> {
        let deadline = self.deadline(now_ms)?;
        if self.revoke_barrier || self.queue.len() == CAPACITY {
            return Err(Error::Busy);
        }
        Ok(deadline)
    }

    pub fn capture(&mut self, now_ms: u64) -> Result<Notice, Error> {
        let deadline_ms = self.admit(now_ms)?;
        let previous_sequence = self.state.notice().sequence;
        let notice = self.state.capture()?;
        if notice.sequence != previous_sequence {
            self.queue.push_back(Pending {
                notice: notice.clone(),
                deadline_ms,
            });
        }
        Ok(notice)
    }

    pub fn key(
        &mut self,
        scope: &Scope,
        code: &str,
        down: bool,
        repeat: bool,
        now_ms: u64,
    ) -> Result<Option<Notice>, Error> {
        let deadline_ms = self.admit(now_ms)?;
        let notice = self.state.key(scope, code, down, repeat)?;
        if let Some(notice) = &notice {
            self.queue.push_back(Pending {
                notice: notice.clone(),
                deadline_ms,
            });
        }
        Ok(notice)
    }

    /// Supersedes ordinary queued input with a pinned empty-set barrier. Its
    /// original deadline survives repeated revocations, including new reasons.
    pub fn revoke(&mut self, reason: RevokeReason, now_ms: u64) -> Result<Notice, Error> {
        let deadline_ms = self.deadline(now_ms)?;
        if self.revoke_barrier {
            return self.pending().cloned().ok_or(Error::Unavailable);
        }
        let notice = self.state.revoke(reason)?;
        self.queue.clear();
        self.queue.push_back(Pending {
            notice: notice.clone(),
            deadline_ms,
        });
        self.revoke_barrier = true;
        Ok(notice)
    }

    pub fn pending(&self) -> Option<&Notice> {
        self.queue.front().map(|pending| &pending.notice)
    }

    /// Only the provider may acknowledge the exact front. One successful
    /// receipt is retained for transport retry; it cannot consume another item.
    pub fn ack(&mut self, provider_owner: u64, notice: &Notice, now_ms: u64) -> Result<(), Error> {
        if provider_owner != notice.scope.owner {
            return Err(Error::Stale);
        }
        self.check_time(now_ms)?;
        if self.last_receipt.as_ref() == Some(notice) {
            return Ok(());
        }
        let front = self.queue.front().ok_or(Error::Unavailable)?;
        if front.notice != *notice {
            return Err(Error::Stale);
        }
        self.queue.pop_front();
        self.revoke_barrier = false;
        self.last_receipt = Some(notice.clone());
        Ok(())
    }

    pub fn expired(&self, now_ms: u64) -> bool {
        self.queue
            .front()
            .is_some_and(|pending| now_ms >= pending.deadline_ms)
    }

    pub fn notice(&self) -> Notice {
        self.state.notice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{IntentTicket, SourceStamp};

    fn scope() -> Scope {
        Scope {
            owner: 7,
            producer_id: 4,
            input_epoch: 2,
            ticket: IntentTicket {
                owner: 99,
                cycle: 1,
                stamp: SourceStamp {
                    generation: 3,
                    intent_version: 5,
                },
            },
        }
    }

    fn delivery() -> Delivery {
        Delivery::new(State::new(scope(), vec!["KeyX".into()]).unwrap())
    }

    #[test]
    fn rapid_press_and_release_are_delivered_in_order_with_exact_provider_ack() {
        let mut delivery = delivery();
        assert!(delivery.pending().is_none());
        let captured = delivery.capture(10).unwrap();
        let down = delivery
            .key(&scope(), "KeyX", true, false, 11)
            .unwrap()
            .unwrap();
        let up = delivery
            .key(&scope(), "KeyX", false, false, 12)
            .unwrap()
            .unwrap();
        assert_eq!(delivery.ack(99, &captured, 13), Err(Error::Stale));
        assert_eq!(delivery.ack(7, &down, 13), Err(Error::Stale));
        assert_eq!(delivery.pending(), Some(&captured));
        delivery.ack(7, &captured, 14).unwrap();
        delivery.ack(7, &captured, 15).unwrap();
        assert_eq!(delivery.pending(), Some(&down));
        assert_eq!(down.pressed, ["KeyX"]);
        delivery.ack(7, &down, 16).unwrap();
        assert!(up.pressed.is_empty());
        delivery.ack(7, &up, 17).unwrap();
        delivery.ack(7, &up, 18).unwrap();
        assert_eq!(delivery.ack(7, &captured, 19), Err(Error::Unavailable));
    }

    #[test]
    fn full_queue_never_mutates_state_and_revoke_can_replace_it() {
        let mut delivery = delivery();
        delivery.capture(0).unwrap();
        for index in 0..31 {
            delivery
                .key(&scope(), "KeyX", index % 2 == 0, false, 1)
                .unwrap();
        }
        let before = delivery.notice();
        assert_eq!(
            delivery.key(&scope(), "KeyX", false, false, 2),
            Err(Error::Busy)
        );
        assert_eq!(delivery.capture(2), Err(Error::Busy));
        assert_eq!(delivery.notice(), before);
        let empty = delivery.revoke(RevokeReason::Menu, 2).unwrap();
        assert!(empty.pressed.is_empty());
        assert!(!empty.captured);
        assert_eq!(delivery.pending(), Some(&empty));
    }

    #[test]
    fn revoke_barrier_rejects_old_in_flight_ack_and_cannot_be_overwritten() {
        let mut delivery = delivery();
        let old = delivery.capture(0).unwrap();
        let empty = delivery.revoke(RevokeReason::FocusLost, 10).unwrap();
        assert_eq!(delivery.ack(7, &old, 11), Err(Error::Stale));
        assert_eq!(delivery.capture(12), Err(Error::Busy));
        assert_eq!(
            delivery.key(&scope(), "KeyX", true, false, 12),
            Err(Error::Busy)
        );
        assert_eq!(delivery.revoke(RevokeReason::Menu, 999).unwrap(), empty);
        assert!(!delivery.expired(1009));
        assert!(delivery.expired(1010));
        assert_eq!(
            delivery.revoke(RevokeReason::Pause, 1010),
            Err(Error::TimedOut)
        );
        assert_eq!(delivery.ack(7, &empty, 1010), Err(Error::TimedOut));
        assert_eq!(delivery.pending(), Some(&empty));
    }

    #[test]
    fn empty_ack_unlocks_capture_but_receipt_replay_does_not_pop_new_capture() {
        let mut delivery = delivery();
        delivery.capture(0).unwrap();
        let empty = delivery.revoke(RevokeReason::Dialog, 1).unwrap();
        delivery.ack(7, &empty, 2).unwrap();
        let next = delivery.capture(3).unwrap();
        delivery.ack(7, &empty, 4).unwrap();
        assert_eq!(delivery.pending(), Some(&next));
        let mut altered = next.clone();
        altered.pressed.push("KeyX".into());
        assert_eq!(delivery.ack(7, &altered, 5), Err(Error::Stale));
        assert_eq!(delivery.pending(), Some(&next));
    }

    #[test]
    fn invalid_time_stale_scope_and_noops_preserve_state_and_queue() {
        let mut delivery = delivery();
        let initial = delivery.notice();
        assert_eq!(delivery.capture(MAX_SAFE - 999), Err(Error::Invalid));
        assert_eq!(delivery.notice(), initial);
        assert!(delivery.pending().is_none());
        let captured = delivery.capture(0).unwrap();
        assert_eq!(delivery.capture(1).unwrap(), captured);
        assert_eq!(delivery.key(&scope(), "KeyX", true, true, 2).unwrap(), None);
        let mut stale = scope();
        stale.input_epoch += 1;
        assert_eq!(
            delivery.key(&stale, "KeyX", true, false, 3),
            Err(Error::Stale)
        );
        assert_eq!(delivery.notice(), captured);
        delivery.ack(7, &captured, 4).unwrap();
        assert!(delivery.pending().is_none());
    }
}
