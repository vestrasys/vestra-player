//! Single-source provider control handshake; not a core-state proof.
//! Source operation receipts, owner cleanup, side effects and clock polling stay
//! in the host adapter. An exact provider echo is distinct from CPAL's buffer ACK.
#![forbid(unsafe_code)]

use crate::playback::session::{IntentTicket, SourceStamp};
use serde::{Deserialize, Serialize};

const MAX_SAFE: u64 = (1_u64 << 53) - 1;
const TIMEOUT_MS: u64 = 5_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Notice {
    pub producer_id: u64,
    pub stamp: SourceStamp,
    pub cycle: u64,
    pub epoch: u64,
    pub base_sample: u64,
    pub paused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub requester: u64,
    pub operation_id: u64,
    pub provider_owner: u64,
    pub ticket: IntentTicket,
    pub notice: Notice,
    pub deadline_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Busy,
    Stale,
    Unavailable,
}

#[derive(Default)]
pub struct Controls {
    pending: Option<Pending>,
}

fn positive(value: u64) -> bool {
    (1..=MAX_SAFE).contains(&value)
}

impl Controls {
    // The explicit argument list is the finite adapter interface, not a generic
    // transaction/options builder. All request fields are checked before store.
    #[allow(clippy::too_many_arguments)]
    pub fn begin(
        &mut self,
        requester: u64,
        operation_id: u64,
        provider_owner: u64,
        producer_id: u64,
        ticket: IntentTicket,
        epoch: u64,
        base_sample: u64,
        paused: bool,
        now_ms: u64,
    ) -> Result<Notice, Error> {
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        let deadline_ms = now_ms
            .checked_add(TIMEOUT_MS)
            .filter(|deadline| *deadline <= MAX_SAFE)
            .ok_or(Error::Invalid)?;
        if ![
            requester,
            operation_id,
            provider_owner,
            producer_id,
            ticket.owner,
            ticket.stamp.generation,
            ticket.stamp.intent_version,
            ticket.cycle,
            epoch,
        ]
        .into_iter()
        .all(positive)
            || ticket.owner != requester
            || base_sample > MAX_SAFE
        {
            return Err(Error::Invalid);
        }
        let notice = Notice {
            producer_id,
            stamp: ticket.stamp,
            cycle: ticket.cycle,
            epoch,
            base_sample,
            paused,
        };
        self.pending = Some(Pending {
            requester,
            operation_id,
            provider_owner,
            ticket,
            notice: notice.clone(),
            deadline_ms,
        });
        Ok(notice)
    }

    /// Only the provider instance receives its outstanding notice. A controller
    /// being disabled does not by itself cancel the provider's pending action.
    pub fn notice_for(&self, provider_owner: u64) -> Option<Notice> {
        self.pending
            .as_ref()
            .filter(|pending| pending.provider_owner == provider_owner)
            .map(|pending| pending.notice.clone())
    }

    /// The adapter must expire using its current monotonic time before handling
    /// an ACK; this pure method has no clock with which to detect a late packet.
    pub fn ack(
        &mut self,
        provider_owner: u64,
        notice: &Notice,
        current: SourceStamp,
    ) -> Result<Pending, Error> {
        let pending = self.pending.as_ref().ok_or(Error::Unavailable)?;
        if pending.provider_owner != provider_owner
            || pending.notice != *notice
            || pending.ticket.stamp != current
        {
            return Err(Error::Stale);
        }
        // No effect/receipt is published here. The serial host arbitration point
        // takes this record and finishes the requester's original operation.
        self.pending.take().ok_or(Error::Unavailable)
    }

    /// Caller supplies monotonic milliseconds; equality is already expired.
    pub fn expire(&mut self, now_ms: u64) -> Option<Pending> {
        if self.pending.as_ref()?.deadline_ms <= now_ms {
            self.pending.take()
        } else {
            None
        }
    }

    /// Source replacement/provider death explicitly invokes this; it does not
    /// run implicitly from the controller's lifecycle or a failed stale ACK.
    pub fn cancel(&mut self) -> Option<Pending> {
        self.pending.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket() -> IntentTicket {
        IntentTicket {
            stamp: SourceStamp {
                generation: 5,
                intent_version: 8,
            },
            cycle: 3,
            owner: 11,
        }
    }
    fn begin(controls: &mut Controls, now: u64) -> Notice {
        controls
            .begin(11, 17, 29, 31, ticket(), 4, 4_800, true, now)
            .unwrap()
    }

    #[test]
    fn matching_provider_echo_takes_original_request_once() {
        let mut controls = Controls::default();
        let notice = begin(&mut controls, 0);
        assert_eq!(controls.notice_for(29), Some(notice.clone()));
        assert_eq!(controls.notice_for(11), None);
        let pending = controls.ack(29, &notice, ticket().stamp).unwrap();
        assert_eq!(pending.requester, 11);
        assert_eq!(pending.operation_id, 17);
        assert_eq!(pending.provider_owner, 29);
        assert_eq!(pending.ticket, ticket());
        assert_eq!(pending.notice, notice);
        assert_eq!(pending.deadline_ms, 5_000);
        assert_eq!(
            controls.ack(29, &notice, ticket().stamp),
            Err(Error::Unavailable)
        );
        assert!(controls.expire(u64::MAX).is_none());
    }

    #[test]
    fn wrong_owner_echo_or_current_stamp_never_consumes_pending() {
        let mut controls = Controls::default();
        let notice = begin(&mut controls, 100);
        assert_eq!(controls.ack(30, &notice, ticket().stamp), Err(Error::Stale));
        for current in [
            SourceStamp {
                generation: 6,
                ..ticket().stamp
            },
            SourceStamp {
                intent_version: 9,
                ..ticket().stamp
            },
        ] {
            assert_eq!(controls.ack(29, &notice, current), Err(Error::Stale));
        }
        let mut variants = vec![notice.clone(); 7];
        variants[0].producer_id += 1;
        variants[1].stamp.generation += 1;
        variants[2].stamp.intent_version += 1;
        variants[3].cycle += 1;
        variants[4].epoch += 1;
        variants[5].base_sample += 1;
        variants[6].paused = false;
        for wrong in variants {
            assert_eq!(controls.ack(29, &wrong, ticket().stamp), Err(Error::Stale));
            assert_eq!(controls.notice_for(29), Some(notice.clone()));
        }
        assert!(controls.ack(29, &notice, ticket().stamp).is_ok());
    }

    #[test]
    fn busy_expiration_boundary_and_explicit_cancel_preserve_identity() {
        let mut controls = Controls::default();
        let original = begin(&mut controls, 100);
        assert_eq!(
            controls.begin(11, 18, 29, 31, ticket(), 5, 0, false, 101),
            Err(Error::Busy)
        );
        assert_eq!(controls.notice_for(29), Some(original.clone()));
        assert!(controls.expire(5_099).is_none());
        assert_eq!(controls.expire(5_100).unwrap().notice, original);
        assert!(controls.expire(5_101).is_none());
        let replacement = begin(&mut controls, 200);
        assert_eq!(controls.cancel().unwrap().notice, replacement);
        assert!(controls.cancel().is_none());
        assert_eq!(
            controls.ack(29, &replacement, ticket().stamp),
            Err(Error::Unavailable)
        );
    }

    #[test]
    fn invalid_numbers_and_deadline_overflow_leave_no_pending_record() {
        let mut controls = Controls::default();
        // Every positive identity/stamp field is independently bounded.
        for field in 0..9 {
            for invalid in [0, MAX_SAFE + 1] {
                let mut values = [11, 17, 29, 31, 11, 5, 8, 3, 4];
                values[field] = invalid;
                let t = IntentTicket {
                    owner: values[4],
                    stamp: SourceStamp {
                        generation: values[5],
                        intent_version: values[6],
                    },
                    cycle: values[7],
                };
                assert_eq!(
                    controls.begin(
                        values[0], values[1], values[2], values[3], t, values[8], 0, false, 0
                    ),
                    Err(Error::Invalid)
                );
                assert!(controls.cancel().is_none());
            }
        }
        let wrong_owner = IntentTicket {
            owner: 12,
            ..ticket()
        };
        assert_eq!(
            controls.begin(11, 17, 29, 31, wrong_owner, 4, 0, true, 0),
            Err(Error::Invalid)
        );
        assert_eq!(
            controls.begin(11, 17, 29, 31, ticket(), 4, MAX_SAFE + 1, true, 0),
            Err(Error::Invalid)
        );
        for now in [MAX_SAFE - TIMEOUT_MS + 1, MAX_SAFE, u64::MAX] {
            assert_eq!(
                controls.begin(11, 17, 29, 31, ticket(), 4, 0, true, now),
                Err(Error::Invalid)
            );
            assert!(controls.cancel().is_none());
        }
        controls
            .begin(
                11,
                17,
                29,
                31,
                ticket(),
                MAX_SAFE,
                MAX_SAFE,
                false,
                MAX_SAFE - TIMEOUT_MS,
            )
            .unwrap();
        assert_eq!(controls.expire(MAX_SAFE).unwrap().deadline_ms, MAX_SAFE);
    }

    #[test]
    fn notice_uses_exact_camel_case_fields_for_provider_echo() {
        let mut controls = Controls::default();
        let notice = begin(&mut controls, 0);
        let json = serde_json::to_value(&notice).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"producerId":31,"stamp":{"generation":5,"intentVersion":8},"cycle":3,"epoch":4,"baseSample":4800,"paused":true})
        );
        assert_eq!(
            serde_json::from_value::<Notice>(json.clone()).unwrap(),
            notice
        );
        let mut extra = json;
        extra["callbackAck"] = true.into();
        assert!(serde_json::from_value::<Notice>(extra).is_err());
    }
}
