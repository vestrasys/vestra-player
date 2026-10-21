//! Per-owner processing-operation retention (host-internal).
//! The caller validates and bounds keys/payloads before admission, executes only
//! Execute, and drops this book when its owner generation ends. No persistence,
//! backend work, resource lifecycle or retry scheduling belongs to this module.
#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};

use serde_json::Value as Json;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    Video,
    Audio,
    Subtitles,
    Media,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    pub stage: Stage,
    pub resource_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub code: String,
    pub message: String,
}

pub type Outcome = Result<Json, Failure>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Admission {
    Execute,
    Replay(Outcome),
    Pending,
    Conflict,
    UnknownExpired,
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    Pending,
    Finished(Outcome),
    UnknownExpired,
    Unknown,
}

struct Record {
    payload: Json,
    outcome: Option<Outcome>,
}

#[derive(Default)]
pub struct Book {
    // Keep one high-water mark per ever-admitted key, including removed
    // resources. Never discard it while the owner can submit another request.
    high_water: BTreeMap<Key, u64>,
    records: BTreeMap<(Key, u64), Record>,
    terminal: VecDeque<(Key, u64)>,
    pending: usize,
}

impl Book {
    pub const MAX_KEYS: usize = 64;
    pub const MAX_PENDING: usize = 128;
    pub const MAX_TERMINAL: usize = 512;
    pub const MAX_REVISION: u64 = (1_u64 << 53) - 1;

    pub fn new() -> Self {
        Self::default()
    }

    /// Full is a definite refusal and does not consume a revision or key slot.
    /// UnknownExpired is an unknown historical result and MUST NOT execute.
    pub fn admit(&mut self, key: Key, revision: u64, payload: Json) -> Admission {
        if revision == 0 || revision > Self::MAX_REVISION {
            return Admission::Conflict;
        }
        if let Some(record) = self.records.get(&(key.clone(), revision)) {
            if record.payload != payload {
                return Admission::Conflict;
            }
            return match &record.outcome {
                Some(outcome) => Admission::Replay(outcome.clone()),
                None => Admission::Pending,
            };
        }
        match self.high_water.get(&key) {
            Some(high_water) if revision <= *high_water => return Admission::UnknownExpired,
            None if self.high_water.len() >= Self::MAX_KEYS => return Admission::Full,
            _ => {}
        }
        if self.pending >= Self::MAX_PENDING {
            return Admission::Full;
        }
        self.high_water.insert(key.clone(), revision);
        self.records.insert(
            (key, revision),
            Record {
                payload,
                outcome: None,
            },
        );
        self.pending += 1;
        Admission::Execute
    }

    /// A pending record may finish once. Duplicate finish is a conflict even if
    /// the result matches; a delayed completion cannot replace a terminal fact.
    pub fn finish(&mut self, key: &Key, revision: u64, outcome: Outcome) -> Result<(), Failure> {
        let record_key = (key.clone(), revision);
        let Some(record) = self.records.get_mut(&record_key) else {
            let expired = self
                .high_water
                .get(key)
                .is_some_and(|high| revision <= *high);
            return Err(Failure {
                code: if expired { "unknownExpired" } else { "missing" }.into(),
                message: if expired {
                    "processing operation is outside retained history; completion cannot be applied"
                } else {
                    "processing operation was never admitted"
                }
                .into(),
            });
        };
        if record.outcome.is_some() {
            return Err(Failure {
                code: "conflict".into(),
                message: "processing operation already has a terminal outcome".into(),
            });
        }
        record.outcome = Some(outcome);
        self.pending -= 1;
        self.terminal.push_back(record_key);
        while self.terminal.len() > Self::MAX_TERMINAL {
            if let Some(expired) = self.terminal.pop_front() {
                self.records.remove(&expired);
            }
        }
        Ok(())
    }

    pub fn query(&self, key: &Key, revision: u64) -> Query {
        if let Some(record) = self.records.get(&(key.clone(), revision)) {
            return match &record.outcome {
                Some(outcome) => Query::Finished(outcome.clone()),
                None => Query::Pending,
            };
        }
        if self
            .high_water
            .get(key)
            .is_some_and(|high| revision <= *high)
        {
            Query::UnknownExpired
        } else {
            Query::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn key(stage: Stage, id: &str) -> Key {
        Key {
            stage,
            resource_id: id.into(),
        }
    }
    fn payload(value: f64) -> Json {
        json!({"expectedGeneration": 7, "nodes": [{"kind": "volume", "gainDb": value}]})
    }

    #[test]
    fn stages_are_independent_and_same_payload_replays_the_original_result() {
        let mut book = Book::new();
        let video = key(Stage::Video, "same");
        let audio = key(Stage::Audio, "same");
        let original = payload(-6.0);
        assert_eq!(
            book.admit(video.clone(), 1, original.clone()),
            Admission::Execute
        );
        assert_eq!(
            book.admit(audio.clone(), 1, original.clone()),
            Admission::Execute
        );
        assert_eq!(
            book.admit(video.clone(), 1, original.clone()),
            Admission::Pending
        );
        assert_eq!(
            book.admit(video.clone(), 1, payload(-3.0)),
            Admission::Conflict
        );
        let result = Err(Failure {
            code: "backend".into(),
            message: "candidate rejected".into(),
        });
        book.finish(&video, 1, result.clone()).unwrap();
        assert_eq!(
            book.admit(video.clone(), 1, original.clone()),
            Admission::Replay(result.clone())
        );
        assert_eq!(book.query(&audio, 1), Query::Pending);
        assert_eq!(
            book.finish(&video, 1, Ok(json!({"applied": true})))
                .unwrap_err()
                .code,
            "conflict"
        );
        assert_eq!(book.query(&video, 1), Query::Finished(result));
        let mut changed_source = original;
        changed_source["expectedGeneration"] = json!(8);
        assert_eq!(book.admit(video, 1, changed_source), Admission::Conflict);
    }

    #[test]
    fn pending_capacity_refusal_does_not_advance_revision_or_allocate_new_key() {
        let mut book = Book::new();
        let key = key(Stage::Audio, "slider");
        for revision in 1..=128 {
            assert_eq!(
                book.admit(key.clone(), revision, payload(0.0)),
                Admission::Execute
            );
        }
        assert_eq!(book.admit(key.clone(), 129, payload(0.0)), Admission::Full);
        assert_eq!(book.high_water[&key], 128);
        let other = super::Key {
            stage: Stage::Video,
            resource_id: "other".into(),
        };
        assert_eq!(book.admit(other.clone(), 1, json!({})), Admission::Full);
        assert!(!book.high_water.contains_key(&other));
        assert_eq!(book.query(&key, 129), Query::Unknown);
        book.finish(&key, 1, Ok(json!("accepted"))).unwrap();
        assert_eq!(book.admit(key, 129, payload(0.0)), Admission::Execute);
        assert_eq!(book.pending, 128);
    }

    #[test]
    fn terminal_eviction_preserves_high_water_and_cannot_repeat_side_effects() {
        let mut book = Book::new();
        let key = key(Stage::Video, "tone");
        for revision in 1..=600 {
            assert_eq!(
                book.admit(key.clone(), revision, json!({"value": revision})),
                Admission::Execute
            );
            book.finish(&key, revision, Ok(json!({"revision": revision})))
                .unwrap();
        }
        assert_eq!(book.records.len(), Book::MAX_TERMINAL);
        assert_eq!(book.terminal.len(), Book::MAX_TERMINAL);
        assert_eq!(book.query(&key, 1), Query::UnknownExpired);
        assert_eq!(
            book.admit(key.clone(), 1, json!({"value": 1})),
            Admission::UnknownExpired
        );
        assert_eq!(
            book.finish(&key, 1, Ok(Json::Null)).unwrap_err().code,
            "unknownExpired"
        );
        assert_eq!(book.query(&key, 601), Query::Unknown);
        assert_eq!(book.high_water[&key], 600);
        assert_eq!(book.pending, 0);
    }

    #[test]
    fn lifetime_key_limit_bounds_tombstones_even_when_results_are_evicted() {
        let mut book = Book::new();
        for index in 0..Book::MAX_KEYS {
            let key = key(Stage::Audio, &format!("r{index}"));
            assert_eq!(book.admit(key.clone(), 1, Json::Null), Admission::Execute);
            book.finish(&key, 1, Ok(Json::Null)).unwrap();
        }
        let extra = key(Stage::Video, "r0");
        assert_eq!(book.admit(extra.clone(), 1, Json::Null), Admission::Full);
        assert_eq!(book.query(&extra, 1), Query::Unknown);
        let retained = key(Stage::Audio, "r0");
        for revision in 2..=600 {
            assert_eq!(
                book.admit(retained.clone(), revision, Json::Null),
                Admission::Execute
            );
            book.finish(&retained, revision, Ok(Json::Null)).unwrap();
        }
        assert_eq!(book.high_water.len(), Book::MAX_KEYS);
        assert_eq!(book.admit(extra, 1, Json::Null), Admission::Full);
        assert!(book.records.len() <= Book::MAX_PENDING + Book::MAX_TERMINAL);
        assert_eq!(
            book.query(&key(Stage::Audio, "r63"), 1),
            Query::UnknownExpired
        );
    }

    #[test]
    fn invalid_and_out_of_order_revisions_do_not_mutate_admitted_facts() {
        let mut book = Book::new();
        let key = key(Stage::Video, "tone");
        for revision in [0, Book::MAX_REVISION + 1, u64::MAX] {
            assert_eq!(
                book.admit(key.clone(), revision, Json::Null),
                Admission::Conflict
            );
        }
        assert!(book.high_water.is_empty());
        assert_eq!(book.admit(key.clone(), 10, Json::Null), Admission::Execute);
        assert_eq!(
            book.admit(key.clone(), 9, Json::Null),
            Admission::UnknownExpired
        );
        assert_eq!(book.query(&key, 9), Query::UnknownExpired);
        assert_eq!(book.query(&key, 11), Query::Unknown);
        assert_eq!(book.query(&key, 10), Query::Pending);
        assert_eq!(book.pending, 1);
    }
}
