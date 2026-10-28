//! Finite physical-input state (host-internal).
//! Key names stay distinct (including both Shift keys). Mapping to game buttons
//! belongs to the provider. This module neither sends nor acknowledges notices
//! and does not itself guarantee reliable delivery or native keyboard capture.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::playback::session::IntentTicket;

const MAX_SAFE: u64 = (1_u64 << 53) - 1;

/// Finite public-winit physical identifiers supported by this host adapter.
/// No text/layout or provider-specific logical-button mapping is performed.
pub fn supported_code(code: &str) -> bool {
    matches!(
        code,
        "ArrowUp"
            | "ArrowDown"
            | "ArrowLeft"
            | "ArrowRight"
            | "ShiftLeft"
            | "ShiftRight"
            | "ControlLeft"
            | "ControlRight"
            | "AltLeft"
            | "AltRight"
            | "Enter"
            | "Space"
            | "Tab"
            | "Backspace"
            | "Delete"
            | "Insert"
            | "Home"
            | "End"
            | "PageUp"
            | "PageDown"
    ) || code
        .strip_prefix("Key")
        .is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_uppercase())
        || code
            .strip_prefix("Digit")
            .is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scope {
    /// Provider instance, independently of the source ticket's requesting owner.
    pub owner: u64,
    pub producer_id: u64,
    pub ticket: IntentTicket,
    pub input_epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RevokeReason {
    FocusLost,
    WindowHidden,
    Menu,
    Dialog,
    Pause,
    SourceChanged,
    ProviderGone,
    Disabled,
    Reconfigured,
    TransportFailure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Notice {
    pub scope: Scope,
    pub sequence: u64,
    pub pressed: Vec<String>,
    pub captured: bool,
    pub reason: Option<RevokeReason>,
}

/// Internal UI admission snapshot. Source arbitration remains on the backend.
#[derive(Clone)]
pub struct Routing {
    pub scope: Scope,
    pub keys: Vec<String>,
    pub captured: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Stale,
    Unavailable,
    Exhausted,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "invalid input scope or physical key declaration",
            Self::Stale => "input scope changed",
            Self::Unavailable => "input scope is not captured",
            Self::Exhausted => "input notice sequence exhausted",
        })
    }
}
impl std::error::Error for Error {}

pub struct State {
    scope: Scope,
    keys: BTreeSet<String>,
    pressed: BTreeSet<String>,
    sequence: u64,
    captured: bool,
    reason: Option<RevokeReason>,
}

impl State {
    pub fn new(scope: Scope, keys: Vec<String>) -> Result<Self, Error> {
        let identities = [
            scope.owner,
            scope.producer_id,
            scope.input_epoch,
            scope.ticket.owner,
            scope.ticket.cycle,
            scope.ticket.stamp.generation,
            scope.ticket.stamp.intent_version,
        ];
        if identities
            .iter()
            .any(|value| !(1..=MAX_SAFE).contains(value))
            || !(1..=32).contains(&keys.len())
        {
            return Err(Error::Invalid);
        }
        let mut declared = BTreeSet::new();
        for key in keys {
            if key.is_empty()
                || key.len() > 32
                || !key.bytes().all(|byte| byte.is_ascii_alphanumeric())
                || !declared.insert(key)
            {
                return Err(Error::Invalid);
            }
        }
        // The native adapter must check these names against real physical
        // KeyCodes. ASCII shape validation is not a claim a key exists.
        Ok(Self {
            scope,
            keys: declared,
            pressed: BTreeSet::new(),
            sequence: 0,
            captured: false,
            reason: None,
        })
    }

    fn next_sequence(&self) -> Result<u64, Error> {
        self.sequence
            .checked_add(1)
            .filter(|next| *next <= MAX_SAFE)
            .ok_or(Error::Exhausted)
    }

    pub fn capture(&mut self) -> Result<Notice, Error> {
        if self.captured {
            return Ok(self.notice());
        }
        let sequence = self.next_sequence()?;
        self.pressed.clear();
        self.sequence = sequence;
        self.captured = true;
        self.reason = None;
        Ok(self.notice())
    }

    pub fn key(
        &mut self,
        scope: &Scope,
        code: &str,
        down: bool,
        repeat: bool,
    ) -> Result<Option<Notice>, Error> {
        if *scope != self.scope {
            return Err(Error::Stale);
        }
        if !self.captured {
            return Err(Error::Unavailable);
        }
        if !self.keys.contains(code) || (down && repeat) || self.pressed.contains(code) == down {
            return Ok(None);
        }
        let sequence = self.next_sequence()?;
        if down {
            self.pressed.insert(code.into());
        } else {
            self.pressed.remove(code);
        }
        self.sequence = sequence;
        self.reason = None;
        Ok(Some(self.notice()))
    }

    /// Every explicit revocation has its own ordered empty-set notice, even
    /// when already uncaptured. It never automatically recaptures input.
    pub fn revoke(&mut self, reason: RevokeReason) -> Result<Notice, Error> {
        let sequence = self.next_sequence()?;
        self.pressed.clear();
        self.sequence = sequence;
        self.captured = false;
        self.reason = Some(reason);
        Ok(self.notice())
    }

    pub fn notice(&self) -> Notice {
        Notice {
            scope: self.scope.clone(),
            sequence: self.sequence,
            pressed: self.pressed.iter().cloned().collect(),
            captured: self.captured,
            reason: self.reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::session::SourceStamp;

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
    fn state() -> State {
        State::new(
            scope(),
            vec!["ShiftRight".into(), "KeyX".into(), "ShiftLeft".into()],
        )
        .unwrap()
    }

    #[test]
    fn contract_reconfigured_replaces_provider_business_reasons() {
        assert_eq!(
            serde_json::to_string(&RevokeReason::Reconfigured).unwrap(),
            "\"reconfigured\""
        );
        for old in ["reset", "loadState"] {
            assert!(serde_json::from_value::<RevokeReason>(serde_json::json!(old)).is_err());
        }
    }

    #[test]
    fn both_shift_keys_remain_independent_and_full_pressed_set_is_sorted() {
        let mut state = state();
        assert_eq!(state.notice().sequence, 0);
        state.capture().unwrap();
        state.key(&scope(), "ShiftRight", true, false).unwrap();
        state.key(&scope(), "ShiftLeft", true, false).unwrap();
        let pressed = state.key(&scope(), "KeyX", true, false).unwrap().unwrap();
        assert_eq!(pressed.pressed, ["KeyX", "ShiftLeft", "ShiftRight"]);
        let remaining = state
            .key(&scope(), "ShiftLeft", false, false)
            .unwrap()
            .unwrap();
        assert_eq!(remaining.pressed, ["KeyX", "ShiftRight"]);
        assert_eq!(remaining.scope.owner, 7);
        assert_eq!(
            remaining.scope.ticket.owner, 99,
            "controller and provider need not match"
        );
    }

    #[test]
    fn repeats_duplicates_undeclared_editor_keys_and_stale_scope_do_not_change_state() {
        let mut state = state();
        let captured = state.capture().unwrap();
        assert_eq!(state.capture().unwrap(), captured);
        for (code, down, repeat) in [
            ("KeyX", true, true),
            ("KeyX", false, false),
            ("Space", true, false),
        ] {
            assert_eq!(state.key(&scope(), code, down, repeat).unwrap(), None);
        }
        assert_eq!(state.notice(), captured);
        state.key(&scope(), "KeyX", true, false).unwrap();
        let before = state.notice();
        assert_eq!(state.key(&scope(), "KeyX", true, false).unwrap(), None);
        let mut stale = scope();
        stale.input_epoch += 1;
        assert_eq!(state.key(&stale, "KeyX", false, false), Err(Error::Stale));
        assert_eq!(state.notice(), before);
    }

    #[test]
    fn revoke_clears_all_and_requires_new_explicit_capture() {
        let mut state = state();
        state.capture().unwrap();
        state.key(&scope(), "KeyX", true, false).unwrap();
        state.key(&scope(), "ShiftRight", true, false).unwrap();
        let revoked = state.revoke(RevokeReason::Menu).unwrap();
        assert!(revoked.pressed.is_empty());
        assert!(!revoked.captured);
        assert_eq!(revoked.reason, Some(RevokeReason::Menu));
        assert_eq!(
            state.key(&scope(), "KeyX", true, false),
            Err(Error::Unavailable)
        );
        assert_eq!(state.notice(), revoked);
        let again = state.revoke(RevokeReason::FocusLost).unwrap();
        assert_eq!(again.sequence, revoked.sequence + 1);
        let captured = state.capture().unwrap();
        assert!(captured.pressed.is_empty());
        assert!(captured.captured);
        assert_eq!(captured.reason, None);
        assert_eq!(serde_json::to_value(revoked).unwrap()["reason"], "menu");
    }

    #[test]
    fn invalid_declarations_and_identities_are_rejected() {
        for keys in [
            vec![],
            vec!["KeyX".into(); 2],
            vec!["x".repeat(33)],
            vec!["Key-X".into()],
            vec!["键".into()],
            vec!["".into()],
            (0..33).map(|n| format!("Key{n}")).collect(),
        ] {
            assert!(matches!(State::new(scope(), keys), Err(Error::Invalid)));
        }
        for bad in [0, MAX_SAFE + 1] {
            for index in 0..7 {
                let mut scope = scope();
                match index {
                    0 => scope.owner = bad,
                    1 => scope.producer_id = bad,
                    2 => scope.input_epoch = bad,
                    3 => scope.ticket.owner = bad,
                    4 => scope.ticket.cycle = bad,
                    5 => scope.ticket.stamp.generation = bad,
                    _ => scope.ticket.stamp.intent_version = bad,
                }
                assert!(matches!(
                    State::new(scope, vec!["KeyX".into()]),
                    Err(Error::Invalid)
                ));
            }
        }
    }

    #[test]
    fn exhausted_mutations_preserve_every_prior_fact() {
        let mut state = state();
        state.sequence = MAX_SAFE;
        let before = state.notice();
        assert_eq!(state.capture(), Err(Error::Exhausted));
        assert_eq!(state.revoke(RevokeReason::Disabled), Err(Error::Exhausted));
        assert_eq!(state.notice(), before);
        state.captured = true;
        state.pressed.insert("ShiftRight".into());
        let before = state.notice();
        assert_eq!(
            state.key(&scope(), "KeyX", true, false),
            Err(Error::Exhausted)
        );
        assert_eq!(
            state.key(&scope(), "ShiftRight", false, false),
            Err(Error::Exhausted)
        );
        assert_eq!(state.revoke(RevokeReason::Dialog), Err(Error::Exhausted));
        assert_eq!(
            state.capture().unwrap(),
            before,
            "idempotent capture needs no new sequence"
        );
        assert_eq!(state.notice(), before);
    }
}
