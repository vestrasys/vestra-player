//! Finite subtitle state (host-internal).
//! The caller parses SRT, verifies its SHA-256, authenticates live owners and
//! applies the backend. This module only checks limits/metadata and arbitrates
//! immutable resource identity and public selection. No rendering or SRT parsing.
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub generation: u64,
    pub revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Track {
    Embedded(i64),
    External {
        owner: u64,
        resource_id: String,
        content_revision: u64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub bytes: Arc<[u8]>,
    pub digest: String,
    pub cues: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    pub owner: u64,
    pub resource_id: String,
    pub revision: u64,
    pub content: Content,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub stamp: Stamp,
    pub selected: Option<Track>,
    /// The secondary subtitle (mpv `secondary-sid`), never the same as `selected`.
    pub secondary: Option<Track>,
    pub resources: Vec<Resource>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Stale,
    Conflict,
    Limit,
    Missing,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid subtitle metadata",
            Self::Stale => "stale subtitle source or selection stamp",
            Self::Conflict => "subtitle revision conflicts with current state",
            Self::Limit => "subtitle resource or counter limit exceeded",
            Self::Missing => "subtitle source, resource or track is missing",
        })
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;
type Key = (u64, String);
#[derive(Clone)]
struct Baseline {
    owner: u64,
    track: Option<Track>,
}

#[derive(Clone)]
pub struct Registry {
    initialized: bool,
    stamp: Stamp,
    embedded: BTreeSet<i64>,
    selected: Option<Track>,
    secondary: Option<Track>,
    resources: BTreeMap<Key, Resource>,
    high_water: BTreeMap<Key, u64>,
    baseline: Option<Baseline>,
}
impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub const MAX_CONTENT_BYTES: usize = 2 * 1024 * 1024;
    pub const MAX_TOTAL_BYTES: usize = 16 * 1024 * 1024;
    pub const MAX_RESOURCES: usize = 16;
    pub const MAX_OWNER_RESOURCES: usize = 4;
    pub const MAX_KEYS: usize = 128;
    pub const MAX_OWNER_KEYS: usize = 16;
    pub const MAX_REVISION: u64 = (1_u64 << 53) - 1;

    pub fn new() -> Self {
        Self {
            initialized: false,
            stamp: Stamp {
                generation: 0,
                revision: 0,
            },
            embedded: BTreeSet::new(),
            selected: None,
            secondary: None,
            resources: BTreeMap::new(),
            high_water: BTreeMap::new(),
            baseline: None,
        }
    }

    /// Same generation is accepted only for the first initialization of the new
    /// empty registry (generation 0). Every subsequent source reset must advance
    /// generation. Reset clears source-local resources and revision tombstones;
    /// expected-generation validation then rejects all previous-source requests.
    pub fn reset_source(
        &mut self,
        generation: u64,
        embedded: Vec<i64>,
        selected: Option<i64>,
    ) -> Result<Snapshot> {
        if generation < self.stamp.generation
            || (self.initialized && generation == self.stamp.generation)
        {
            return Err(Error::Stale);
        }
        if embedded.len() > 1024 {
            return Err(Error::Limit);
        }
        let count = embedded.len();
        let embedded: BTreeSet<_> = embedded.into_iter().collect();
        if count != embedded.len() || selected.is_some_and(|track| !embedded.contains(&track)) {
            return Err(Error::Invalid);
        }
        self.stamp = Stamp {
            generation,
            revision: 0,
        };
        self.initialized = true;
        self.embedded = embedded;
        self.selected = selected.map(Track::Embedded);
        self.secondary = None;
        self.resources.clear();
        self.high_water.clear();
        self.baseline = None;
        Ok(self.state())
    }

    /// An actual reload at the same source generation invalidates async apply
    /// stamps while retaining immutable external resources and a valid baseline.
    pub fn reload_embedded(&mut self, generation: u64, embedded: Vec<i64>) -> Result<Snapshot> {
        self.generation(generation)?;
        if embedded.len() > 1024 {
            return Err(Error::Limit);
        }
        let count = embedded.len();
        let embedded: BTreeSet<_> = embedded.into_iter().collect();
        if embedded.len() != count {
            return Err(Error::Invalid);
        }
        let revision = self
            .stamp
            .revision
            .checked_add(1)
            .filter(|v| *v <= Self::MAX_REVISION)
            .ok_or(Error::Limit)?;
        if let Some(Track::Embedded(id)) = self.selected
            && !embedded.contains(&id)
        {
            self.selected = None;
            self.baseline = None;
        }
        if let Some(Track::Embedded(id)) = self.secondary
            && !embedded.contains(&id)
        {
            self.secondary = None;
        }
        self.embedded = embedded;
        self.stamp.revision = revision;
        Ok(self.state())
    }
    pub fn state(&self) -> Snapshot {
        Snapshot {
            stamp: self.stamp,
            selected: self.selected.clone(),
            secondary: self.secondary.clone(),
            resources: self.resources.values().cloned().collect(),
        }
    }

    fn generation(&self, generation: u64) -> Result<()> {
        if !self.initialized {
            return Err(Error::Missing);
        }
        if self.stamp.generation != generation {
            return Err(Error::Stale);
        }
        Ok(())
    }
    fn expected(&self, expected: Stamp) -> Result<()> {
        self.generation(expected.generation)?;
        if self.stamp != expected {
            return Err(Error::Stale);
        }
        Ok(())
    }
    fn next_revision(&self) -> Result<u64> {
        self.stamp.revision.checked_add(1).ok_or(Error::Limit)
    }

    fn prepare(&self, owner: u64, id: String, revision: u64, content: Content) -> Result<Resource> {
        if id.is_empty()
            || id.len() > 128
            || revision == 0
            || revision > Self::MAX_REVISION
            || content.bytes.len() > Self::MAX_CONTENT_BYTES
            || !(1..=20_000).contains(&content.cues)
            || std::str::from_utf8(&content.bytes).is_err()
            || content.digest.len() != 64
            || !content
                .digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(Error::Invalid);
        }
        let key = (owner, id.clone());
        if self
            .high_water
            .get(&key)
            .is_some_and(|high| revision <= *high)
        {
            return Err(Error::Conflict);
        }
        if !self.high_water.contains_key(&key)
            && (self.high_water.len() >= Self::MAX_KEYS
                || self.high_water.keys().filter(|key| key.0 == owner).count()
                    >= Self::MAX_OWNER_KEYS)
        {
            return Err(Error::Limit);
        }
        if !self.resources.contains_key(&key)
            && (self.resources.len() >= Self::MAX_RESOURCES
                || self.resources.keys().filter(|key| key.0 == owner).count()
                    >= Self::MAX_OWNER_RESOURCES)
        {
            return Err(Error::Limit);
        }
        let mut bytes = content.bytes.len();
        for (existing_key, resource) in &self.resources {
            if *existing_key != key {
                bytes = bytes
                    .checked_add(resource.content.bytes.len())
                    .ok_or(Error::Limit)?;
            }
        }
        if bytes > Self::MAX_TOTAL_BYTES {
            return Err(Error::Limit);
        }
        Ok(Resource {
            owner,
            resource_id: id,
            revision,
            content,
        })
    }

    fn insert(&mut self, resource: Resource) {
        let key = (resource.owner, resource.resource_id.clone());
        self.high_water.insert(key.clone(), resource.revision);
        self.resources.insert(key, resource);
    }
    fn valid_track(&self, target: &Track) -> bool {
        match target {
            Track::Embedded(id) => self.embedded.contains(id),
            Track::External {
                owner,
                resource_id,
                content_revision,
            } => self
                .resources
                .get(&(*owner, resource_id.clone()))
                .is_some_and(|resource| resource.revision == *content_revision),
        }
    }
    fn selected_key(&self) -> Option<Key> {
        match &self.selected {
            Some(Track::External {
                owner, resource_id, ..
            }) => Some((*owner, resource_id.clone())),
            _ => None,
        }
    }
    fn commit_selection(&mut self, caller: u64, target: Option<Track>, revision: u64) {
        let target_owner = match &target {
            Some(Track::External { owner, .. }) => Some(*owner),
            _ => None,
        };
        if target_owner == Some(caller) {
            let keep = self
                .baseline
                .as_ref()
                .is_some_and(|baseline| baseline.owner == caller)
                && self.selected_key().is_some_and(|key| key.0 == caller);
            if !keep {
                self.baseline = Some(Baseline {
                    owner: caller,
                    track: self.selected.clone(),
                });
            }
        } else {
            // Explicit selection by another caller revokes the previous owner's
            // restore eligibility even when the selected target is unchanged.
            self.baseline = None;
        }
        self.selected = target;
        self.stamp.revision = revision;
    }

    pub fn register(
        &mut self,
        owner: u64,
        id: String,
        revision: u64,
        content: Content,
        expected_generation: u64,
    ) -> Result<Snapshot> {
        self.generation(expected_generation)?;
        if self.selected_key() == Some((owner, id.clone())) {
            return Err(Error::Conflict);
        }
        let resource = self.prepare(owner, id, revision, content)?;
        self.insert(resource);
        Ok(self.state())
    }

    pub fn register_and_select(
        &mut self,
        owner: u64,
        id: String,
        revision: u64,
        content: Content,
        expected: Stamp,
    ) -> Result<Snapshot> {
        self.expected(expected)?;
        let resource = self.prepare(owner, id, revision, content)?;
        let next = self.next_revision()?;
        let target = Track::External {
            owner,
            resource_id: resource.resource_id.clone(),
            content_revision: revision,
        };
        self.insert(resource);
        self.commit_selection(owner, Some(target), next);
        Ok(self.state())
    }

    /// Every accepted intent advances selection revision, including same-target
    /// selection and closing an already closed track. Any live caller may select
    /// a public external track, but only selecting one's own resource grants a
    /// baseline restore eligibility to that resource owner.
    pub fn select(
        &mut self,
        owner: u64,
        target: Option<Track>,
        expected: Stamp,
    ) -> Result<Snapshot> {
        self.expected(expected)?;
        if target
            .as_ref()
            .is_some_and(|track| !self.valid_track(track))
        {
            return Err(Error::Missing);
        }
        if target.is_some() && target == self.secondary {
            return Err(Error::Invalid);
        }
        let next = self.next_revision()?;
        self.commit_selection(owner, target, next);
        Ok(self.state())
    }

    /// R-T3: the secondary subtitle; not the primary track, and the caller
    /// checks it is a text track. Advances the selection revision.
    pub fn select_secondary(&mut self, target: Option<Track>, expected: Stamp) -> Result<Snapshot> {
        self.expected(expected)?;
        if let Some(track) = &target {
            if !self.valid_track(track) {
                return Err(Error::Missing);
            }
            if self.selected.as_ref() == Some(track) {
                return Err(Error::Invalid);
            }
        }
        self.stamp.revision = self.next_revision()?;
        self.secondary = target;
        Ok(self.state())
    }

    /// Upload and make it the secondary subtitle in one step.
    pub fn register_as_secondary(
        &mut self,
        owner: u64,
        id: String,
        revision: u64,
        content: Content,
        expected: Stamp,
    ) -> Result<Snapshot> {
        self.expected(expected)?;
        let resource = self.prepare(owner, id, revision, content)?;
        let next = self.next_revision()?;
        self.secondary = Some(Track::External {
            owner,
            resource_id: resource.resource_id.clone(),
            content_revision: revision,
        });
        self.insert(resource);
        self.stamp.revision = next;
        Ok(self.state())
    }

    /// The secondary goes with its resource.
    fn drop_secondary(&mut self, gone: impl Fn(&Key) -> bool) -> bool {
        let gone = matches!(&self.secondary, Some(Track::External { owner, resource_id, .. })
            if gone(&(*owner, resource_id.clone())));
        if gone {
            self.secondary = None;
        }
        gone
    }

    fn restore_target(&self, owner: u64, revoking_owner: bool) -> Option<Track> {
        let baseline = self
            .baseline
            .as_ref()
            .filter(|baseline| baseline.owner == owner)?;
        let track = baseline.track.as_ref()?;
        if revoking_owner
            && matches!(track, Track::External { owner: target_owner, .. } if *target_owner == owner)
        {
            return None;
        }
        self.valid_track(track).then(|| track.clone())
    }

    pub fn remove_resource(&mut self, owner: u64, id: &str, expected: Stamp) -> Result<Snapshot> {
        self.expected(expected)?;
        let key = (owner, id.to_owned());
        if !self.resources.contains_key(&key) {
            return Err(Error::Missing);
        }
        let selected = self.selected_key() == Some(key.clone());
        if self.drop_secondary(|each| *each == key) && !selected {
            self.stamp.revision = self.next_revision()?;
        }
        let next = if selected {
            Some(self.next_revision()?)
        } else {
            None
        };
        self.resources.remove(&key);
        if let Some(next) = next {
            self.selected = self.restore_target(owner, false);
            self.baseline = None;
            self.stamp.revision = next;
        }
        Ok(self.state())
    }

    /// Runtime must reject further calls from a revoked owner generation. Its
    /// key high-water marks may then be dropped without an unbounded dead-owner
    /// history. No other owner's current selection or high-water marks change.
    pub fn remove_owner(&mut self, owner: u64) -> Result<Snapshot> {
        let selected = self.selected_key().is_some_and(|key| key.0 == owner);
        if self.drop_secondary(|key| key.0 == owner) && !selected {
            self.stamp.revision = self.next_revision()?;
        }
        let next = if selected {
            Some(self.next_revision()?)
        } else {
            None
        };
        self.resources.retain(|key, _| key.0 != owner);
        self.high_water.retain(|key, _| key.0 != owner);
        if let Some(next) = next {
            self.selected = self.restore_target(owner, true);
            self.baseline = None;
            self.stamp.revision = next;
        } else if self
            .baseline
            .as_ref()
            .is_some_and(|baseline| baseline.owner == owner)
        {
            self.baseline = None;
        }
        Ok(self.state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn content() -> Content {
        Content {
            bytes: Arc::from(&b"caller-validated SRT"[..]),
            digest: "a".repeat(64),
            cues: 1,
        }
    }
    fn fresh_registry() -> Registry {
        let mut registry = Registry::new();
        registry.reset_source(1, vec![10, 20], Some(10)).unwrap();
        registry
    }
    fn external(owner: u64, id: &str, revision: u64) -> Track {
        Track::External {
            owner,
            resource_id: id.into(),
            content_revision: revision,
        }
    }
    fn apply(registry: &mut Registry, owner: u64, id: &str, revision: u64) -> Snapshot {
        registry
            .register_and_select(
                owner,
                id.into(),
                revision,
                content(),
                registry.state().stamp,
            )
            .unwrap()
    }

    #[test]
    fn atomic_validation_and_old_source_requests_preserve_all_current_facts() {
        let mut registry = fresh_registry();
        let current = apply(&mut registry, 1, "original", 1);
        let mut invalid = content();
        invalid.digest = "BAD".into();
        assert_eq!(
            registry.register_and_select(1, "original".into(), 2, invalid, current.stamp),
            Err(Error::Invalid)
        );
        assert_eq!(registry.state(), current);
        assert_eq!(
            registry.register(1, "original".into(), 2, content(), 1),
            Err(Error::Conflict)
        );
        let fresh = registry.reset_source(2, vec![30], Some(30)).unwrap();
        assert_eq!(
            registry.register_and_select(1, "original".into(), 3, content(), current.stamp),
            Err(Error::Stale)
        );
        assert_eq!(
            registry.register(1, "other".into(), 1, content(), 1),
            Err(Error::Stale)
        );
        assert_eq!(registry.state(), fresh);
        assert_eq!(registry.reset_source(2, vec![], None), Err(Error::Stale));
        let mut empty = Registry::new();
        assert!(empty.reset_source(0, vec![], None).is_ok());
        assert_eq!(empty.reset_source(0, vec![], None), Err(Error::Stale));
    }

    #[test]
    fn explicit_off_or_embedded_invalidates_old_apply_and_old_restore() {
        for target in [None, Some(Track::Embedded(20))] {
            let mut registry = fresh_registry();
            let previous = apply(&mut registry, 1, "translation", 1);
            let chosen = registry.select(2, target.clone(), previous.stamp).unwrap();
            assert_eq!(
                registry.register_and_select(1, "translation".into(), 2, content(), previous.stamp),
                Err(Error::Stale)
            );
            let revoked = registry.remove_owner(1).unwrap();
            assert_eq!(revoked.selected, target);
            assert_eq!(revoked.stamp, chosen.stamp);
            assert_eq!(registry.remove_owner(1).unwrap(), revoked);
        }
    }

    #[test]
    fn own_tracks_keep_first_baseline_but_other_caller_same_target_revokes_it() {
        let mut registry = fresh_registry();
        apply(&mut registry, 1, "original", 1);
        apply(&mut registry, 1, "translation", 1);
        let original = registry
            .select(1, Some(external(1, "original", 1)), registry.state().stamp)
            .unwrap();
        assert_eq!(
            registry.remove_owner(1).unwrap().selected,
            Some(Track::Embedded(10))
        );
        assert_eq!(original.selected, Some(external(1, "original", 1)));
        let mut registry = fresh_registry();
        let applied = apply(&mut registry, 1, "a", 1);
        let explicit = registry
            .select(2, applied.selected.clone(), applied.stamp)
            .unwrap();
        assert!(explicit.stamp.revision > applied.stamp.revision);
        assert_eq!(registry.remove_owner(1).unwrap().selected, None);
    }

    #[test]
    fn dead_baseline_is_not_an_undo_stack_and_snapshots_keep_immutable_bytes() {
        let mut registry = fresh_registry();
        let historical = apply(&mut registry, 1, "a", 1);
        apply(&mut registry, 2, "b", 1);
        let before = registry.state().stamp;
        registry.remove_owner(1).unwrap();
        assert_eq!(registry.state().stamp, before);
        assert_eq!(registry.state().selected, Some(external(2, "b", 1)));
        assert_eq!(registry.remove_owner(2).unwrap().selected, None);
        assert_eq!(
            &*historical.resources[0].content.bytes,
            b"caller-validated SRT"
        );
        assert!(registry.state().resources.is_empty());
    }

    #[test]
    fn resource_removal_restores_only_current_valid_baseline_and_keeps_high_water() {
        let mut registry = fresh_registry();
        let selected = apply(&mut registry, 1, "a", 1);
        let removed = registry.remove_resource(1, "a", selected.stamp).unwrap();
        assert_eq!(removed.selected, Some(Track::Embedded(10)));
        assert_eq!(
            registry.register(1, "a".into(), 1, content(), 1),
            Err(Error::Conflict)
        );
        assert!(registry.register(1, "a".into(), 2, content(), 1).is_ok());
        let stamp = registry.state().stamp;
        registry.remove_resource(1, "a", stamp).unwrap();
        assert_eq!(registry.state().stamp, stamp);
    }

    #[test]
    fn live_count_and_byte_limits_fail_without_partial_commit() {
        let mut registry = fresh_registry();
        for owner in 1..=4 {
            for resource in 0..4 {
                registry
                    .register(owner, format!("r{resource}"), 1, content(), 1)
                    .unwrap();
            }
        }
        let full = registry.state();
        assert_eq!(
            registry.register(1, "fifth".into(), 1, content(), 1),
            Err(Error::Limit)
        );
        assert_eq!(
            registry.register(5, "global-extra".into(), 1, content(), 1),
            Err(Error::Limit)
        );
        assert_eq!(registry.state(), full);
        let mut registry = fresh_registry();
        let mut large = content();
        large.bytes = vec![b'x'; Registry::MAX_CONTENT_BYTES].into();
        for owner in 1..=2 {
            for resource in 0..4 {
                registry
                    .register(owner, format!("r{resource}"), 1, large.clone(), 1)
                    .unwrap();
            }
        }
        let full = registry.state();
        assert_eq!(
            registry.register_and_select(3, "over-bytes".into(), 1, content(), full.stamp),
            Err(Error::Limit)
        );
        assert_eq!(registry.state(), full);
    }

    #[test]
    fn metadata_limits_and_repeated_selection_intents_do_not_silently_succeed() {
        let mut registry = fresh_registry();
        let before = registry.state();
        let mut invalid_utf8 = content();
        invalid_utf8.bytes = vec![0xff].into();
        let mut too_large = content();
        too_large.bytes = vec![b'x'; Registry::MAX_CONTENT_BYTES + 1].into();
        let mut zero_cues = content();
        zero_cues.cues = 0;
        let mut many_cues = content();
        many_cues.cues = 20_001;
        let mut uppercase_digest = content();
        uppercase_digest.digest = "A".repeat(64);
        for content in [
            invalid_utf8,
            too_large,
            zero_cues,
            many_cues,
            uppercase_digest,
        ] {
            assert_eq!(
                registry.register_and_select(1, "bad".into(), 1, content, before.stamp),
                Err(Error::Invalid)
            );
            assert_eq!(registry.state(), before);
        }
        assert_eq!(
            registry.register(1, "bad".into(), Registry::MAX_REVISION + 1, content(), 1),
            Err(Error::Invalid)
        );
        let first = registry.select(1, None, before.stamp).unwrap();
        let second = registry.select(2, None, first.stamp).unwrap();
        assert!(second.stamp.revision > first.stamp.revision);
        assert_eq!(
            registry.select(1, Some(Track::Embedded(10)), first.stamp),
            Err(Error::Stale)
        );
        assert_eq!(registry.state(), second);
    }

    #[test]
    fn deleted_resource_keys_are_bounded_until_owner_cleanup_or_source_change() {
        let mut registry = fresh_registry();
        for owner in 1..=8 {
            for resource in 0..16 {
                let id = format!("r{resource}");
                registry
                    .register(owner, id.clone(), 1, content(), 1)
                    .unwrap();
                registry
                    .remove_resource(owner, &id, registry.state().stamp)
                    .unwrap();
            }
        }
        assert_eq!(registry.high_water.len(), Registry::MAX_KEYS);
        assert_eq!(
            registry.register(1, "owner-extra".into(), 1, content(), 1),
            Err(Error::Limit)
        );
        assert_eq!(
            registry.register(9, "global-extra".into(), 1, content(), 1),
            Err(Error::Limit)
        );
        registry.remove_owner(1).unwrap();
        assert_eq!(registry.high_water.len(), 112);
        assert!(
            registry
                .register(9, "global-extra".into(), 1, content(), 1)
                .is_ok()
        );
        registry.reset_source(2, vec![], None).unwrap();
        assert!(registry.high_water.is_empty());
    }

    #[test]
    fn secondary_is_never_the_primary_and_goes_with_its_resource_or_source() {
        let mut registry = fresh_registry(); // embedded 10 (selected), 20
        let stamp = registry.state().stamp;
        assert_eq!(
            registry.select_secondary(Some(Track::Embedded(10)), stamp),
            Err(Error::Invalid)
        );
        let state = registry
            .select_secondary(Some(Track::Embedded(20)), stamp)
            .unwrap();
        assert_eq!(state.secondary, Some(Track::Embedded(20)));
        assert!(state.stamp.revision > stamp.revision);
        assert_eq!(
            registry.select(1, Some(Track::Embedded(20)), registry.state().stamp),
            Err(Error::Invalid),
            "the primary cannot take the secondary track"
        );
        let state = registry
            .register_as_secondary(1, "zh".into(), 1, content(), registry.state().stamp)
            .unwrap();
        assert_eq!(state.secondary, Some(external(1, "zh", 1)));
        assert_eq!(state.selected, Some(Track::Embedded(10)));
        let state = registry
            .remove_resource(1, "zh", registry.state().stamp)
            .unwrap();
        assert_eq!(state.secondary, None);
        registry
            .register_as_secondary(2, "en".into(), 1, content(), registry.state().stamp)
            .unwrap();
        assert_eq!(registry.remove_owner(2).unwrap().secondary, None);
        registry
            .select_secondary(Some(Track::Embedded(20)), registry.state().stamp)
            .unwrap();
        assert_eq!(
            registry.reload_embedded(1, vec![10]).unwrap().secondary,
            None
        );
        registry
            .select_secondary(None, registry.state().stamp)
            .unwrap();
        registry.reset_source(2, vec![1], None).unwrap();
        assert_eq!(registry.state().secondary, None);
    }
}
