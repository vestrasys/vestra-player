//! Experimental bounded in-memory subtitle byte uploads, not a final public API.
//! This does not validate SRT, a digest, base64, or rendering. The caller checks
//! source CAS at begin and authenticates live owner generations. No clocks/files.
#![forbid(unsafe_code)]

use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Status {
    pub generation: u64,
    pub expected_bytes: usize,
    pub received_bytes: usize,
    /// True after successful complete(), not merely after the last append.
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    Invalid,
    Stale,
    Conflict,
    Limit,
    Missing,
    UnknownExpired,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid upload ID, byte count or chunk bounds",
            Self::Stale => "upload belongs to a different source generation",
            Self::Conflict => "upload parameters, byte range or completion state conflict",
            Self::Limit => "upload memory, owner or active-token limit reached",
            Self::Missing => "upload token was not admitted",
            Self::UnknownExpired => "upload token is no longer retained and cannot be recreated",
        })
    }
}
impl std::error::Error for Error {}
type Result<T> = std::result::Result<T, Error>;

pub struct Registry {
    active: BTreeMap<(u64, u64), Upload>,
    high_water: BTreeMap<u64, u64>,
    reserved: usize,
}
struct Upload {
    generation: u64,
    expected: usize,
    storage: Storage,
}
enum Storage {
    Receiving(Vec<u8>),
    Completed(Arc<[u8]>),
}
impl Upload {
    fn status(&self) -> Status {
        let (received_bytes, complete) = match &self.storage {
            Storage::Receiving(bytes) => (bytes.len(), false),
            Storage::Completed(bytes) => (bytes.len(), true),
        };
        Status {
            generation: self.generation,
            expected_bytes: self.expected,
            received_bytes,
            complete,
        }
    }
}
impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    pub const MAX_ID: u64 = (1_u64 << 53) - 1;
    pub const MAX_OWNERS: usize = 64;
    pub const MAX_OWNER_ACTIVE: usize = 2;
    pub const MAX_ACTIVE: usize = 8;
    pub const MAX_UPLOAD_BYTES: usize = 2 * 1024 * 1024;
    pub const MAX_RESERVED_BYTES: usize = 16 * 1024 * 1024;
    pub const MAX_CHUNK_BYTES: usize = 64 * 1024;

    pub fn new() -> Self {
        Self {
            active: BTreeMap::new(),
            high_water: BTreeMap::new(),
            reserved: 0,
        }
    }
    fn valid_id(id: u64) -> Result<()> {
        if id == 0 || id > Self::MAX_ID {
            Err(Error::Invalid)
        } else {
            Ok(())
        }
    }
    fn absent(&self, owner: u64, id: u64) -> Error {
        if self.high_water.get(&owner).is_some_and(|high| id <= *high) {
            Error::UnknownExpired
        } else {
            Error::Missing
        }
    }

    /// An admitted ID cannot be reused after discard/source change. Definite
    /// refusals (including Limit) never consume an owner slot or advance its ID.
    pub fn begin(
        &mut self,
        owner: u64,
        id: u64,
        generation: u64,
        expected_bytes: usize,
    ) -> Result<Status> {
        Self::valid_id(id)?;
        if let Some(upload) = self.active.get(&(owner, id)) {
            return if upload.generation == generation && upload.expected == expected_bytes {
                Ok(upload.status())
            } else {
                Err(Error::Conflict)
            };
        }
        if self.high_water.get(&owner).is_some_and(|high| id <= *high) {
            return Err(Error::UnknownExpired);
        }
        if expected_bytes == 0 || expected_bytes > Self::MAX_UPLOAD_BYTES {
            return Err(Error::Invalid);
        }
        if (!self.high_water.contains_key(&owner) && self.high_water.len() >= Self::MAX_OWNERS)
            || self.active.len() >= Self::MAX_ACTIVE
            || self.active.keys().filter(|key| key.0 == owner).count() >= Self::MAX_OWNER_ACTIVE
        {
            return Err(Error::Limit);
        }
        let reserved = self
            .reserved
            .checked_add(expected_bytes)
            .ok_or(Error::Limit)?;
        if reserved > Self::MAX_RESERVED_BYTES {
            return Err(Error::Limit);
        }
        let upload = Upload {
            generation,
            expected: expected_bytes,
            storage: Storage::Receiving(Vec::with_capacity(expected_bytes)),
        };
        let status = upload.status();
        self.active.insert((owner, id), upload);
        self.high_water.insert(owner, id);
        self.reserved = reserved;
        Ok(status)
    }

    /// Empty/oversized chunks, offset arithmetic overflow and ranges beyond the
    /// declared size are Invalid. Gaps, partial overlap, changed replay bytes and
    /// any append after complete are Conflict. Fully received identical ranges
    /// replay idempotently, returning the current aggregate status.
    pub fn append(
        &mut self,
        owner: u64,
        id: u64,
        current_generation: u64,
        offset: usize,
        chunk: &[u8],
    ) -> Result<Status> {
        Self::valid_id(id)?;
        let absent = self.absent(owner, id);
        let upload = self.active.get_mut(&(owner, id)).ok_or(absent)?;
        if upload.generation != current_generation {
            return Err(Error::Stale);
        }
        let Storage::Receiving(bytes) = &mut upload.storage else {
            return Err(Error::Conflict);
        };
        if chunk.is_empty() || chunk.len() > Self::MAX_CHUNK_BYTES {
            return Err(Error::Invalid);
        }
        let end = offset.checked_add(chunk.len()).ok_or(Error::Invalid)?;
        if end > upload.expected {
            return Err(Error::Invalid);
        }
        if offset == bytes.len() {
            bytes.extend_from_slice(chunk);
        } else if end <= bytes.len() {
            if &bytes[offset..end] != chunk {
                return Err(Error::Conflict);
            }
        } else {
            return Err(Error::Conflict);
        }
        Ok(upload.status())
    }

    /// Completion consumes the Vec into immutable Arc storage once. Subsequent
    /// completion returns a clone of that same allocation. Truncation conflicts
    /// without changing bytes, reserved accounting or completion state.
    pub fn complete(&mut self, owner: u64, id: u64, current_generation: u64) -> Result<Arc<[u8]>> {
        Self::valid_id(id)?;
        let absent = self.absent(owner, id);
        let upload = self.active.get_mut(&(owner, id)).ok_or(absent)?;
        if upload.generation != current_generation {
            return Err(Error::Stale);
        }
        match &mut upload.storage {
            Storage::Completed(bytes) => Ok(bytes.clone()),
            Storage::Receiving(bytes) => {
                if bytes.len() != upload.expected {
                    return Err(Error::Conflict);
                }
                let bytes: Arc<[u8]> = std::mem::take(bytes).into();
                upload.storage = Storage::Completed(bytes.clone());
                Ok(bytes)
            }
        }
    }

    pub fn status(&self, owner: u64, id: u64) -> Result<Status> {
        Self::valid_id(id)?;
        self.active
            .get(&(owner, id))
            .map(Upload::status)
            .ok_or_else(|| self.absent(owner, id))
    }

    /// Valid unknown IDs are harmless no-ops; never allocate a tombstone for an
    /// unadmitted ID. Existing high-water marks survive discard.
    pub fn discard(&mut self, owner: u64, id: u64) -> Result<()> {
        Self::valid_id(id)?;
        if let Some(upload) = self.active.remove(&(owner, id)) {
            self.reserved -= upload.expected;
        }
        Ok(())
    }

    /// Runtime must reject late requests from the stopped owner generation.
    /// Previously returned Arc clones keep immutable bytes independently alive.
    pub fn remove_owner(&mut self, owner: u64) {
        self.active.retain(|key, upload| {
            if key.0 == owner {
                self.reserved -= upload.expected;
                false
            } else {
                true
            }
        });
        self.high_water.remove(&owner);
    }

    /// A source switch discards other active uploads but preserves owner ID
    /// high-water marks, so a delayed begin cannot revive a discarded token.
    pub fn retain_generation(&mut self, generation: u64) {
        self.active.retain(|_, upload| {
            if upload.generation != generation {
                self.reserved -= upload.expected;
                false
            } else {
                true
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_begin_and_received_ranges_replay_but_conflicts_preserve_bytes() {
        let mut registry = Registry::new();
        let initial = registry.begin(1, 1, 7, 6).unwrap();
        assert_eq!(registry.begin(1, 1, 7, 6).unwrap(), initial);
        assert_eq!(registry.begin(1, 1, 8, 6), Err(Error::Conflict));
        assert_eq!(registry.begin(1, 1, 7, 5), Err(Error::Conflict));
        let partial = registry.append(1, 1, 7, 0, b"abc").unwrap();
        assert_eq!(registry.append(1, 1, 7, 0, b"abc").unwrap(), partial);
        assert_eq!(registry.append(1, 1, 7, 1, b"b").unwrap(), partial);
        for (offset, chunk) in [(0, &b"AB"[..]), (2, &b"cd"[..]), (4, &b"e"[..])] {
            assert_eq!(
                registry.append(1, 1, 7, offset, chunk),
                Err(Error::Conflict)
            );
            assert_eq!(registry.status(1, 1).unwrap(), partial);
        }
        assert_eq!(registry.complete(1, 1, 7), Err(Error::Conflict));
        registry.append(1, 1, 7, 3, b"def").unwrap();
        assert!(!registry.status(1, 1).unwrap().complete);
        let bytes = registry.complete(1, 1, 7).unwrap();
        assert_eq!(&*bytes, b"abcdef");
        assert!(Arc::ptr_eq(&bytes, &registry.complete(1, 1, 7).unwrap()));
        assert!(registry.status(1, 1).unwrap().complete);
        assert_eq!(registry.append(1, 1, 7, 0, b"abc"), Err(Error::Conflict));
    }

    #[test]
    fn source_switch_rejects_late_chunks_and_old_begin_cannot_recreate_upload() {
        let mut registry = Registry::new();
        registry.begin(1, 4, 7, 3).unwrap();
        let partial = registry.append(1, 4, 7, 0, b"a").unwrap();
        assert_eq!(registry.append(1, 4, 8, 1, b"b"), Err(Error::Stale));
        assert_eq!(registry.complete(1, 4, 8), Err(Error::Stale));
        assert_eq!(registry.status(1, 4).unwrap(), partial);
        registry.begin(2, 1, 8, 3).unwrap();
        registry.retain_generation(8);
        assert_eq!(registry.status(1, 4), Err(Error::UnknownExpired));
        assert_eq!(registry.begin(1, 4, 7, 3), Err(Error::UnknownExpired));
        assert_eq!(registry.status(1, 5), Err(Error::Missing));
        assert_eq!(registry.reserved, 3);
        assert!(registry.status(2, 1).is_ok());
        assert!(registry.begin(1, 5, 8, 3).is_ok());
    }

    #[test]
    fn owner_cleanup_is_isolated_and_discarded_arcs_keep_original_bytes() {
        let mut registry = Registry::new();
        for owner in [1, 2] {
            registry.begin(owner, 1, 7, 3).unwrap();
        }
        registry.append(1, 1, 7, 0, b"one").unwrap();
        let bytes = registry.complete(1, 1, 7).unwrap();
        registry.discard(1, 1).unwrap();
        registry.discard(1, 1).unwrap();
        registry.discard(99, 42).unwrap();
        assert!(!registry.high_water.contains_key(&99));
        assert_eq!(registry.reserved, 3);
        assert_eq!(&*bytes, b"one");
        assert_eq!(registry.begin(1, 1, 7, 3), Err(Error::UnknownExpired));
        registry.remove_owner(1);
        registry.remove_owner(1);
        assert!(registry.status(2, 1).is_ok());
        assert_eq!(registry.status(1, 1), Err(Error::Missing));
        registry.begin(3, 1, 7, 3).unwrap(); // New live generation can use its own ID 1.
    }

    #[test]
    fn byte_chunk_and_id_validation_never_change_existing_state_or_budget() {
        let mut registry = Registry::new();
        for (id, size) in [
            (0, 1),
            (Registry::MAX_ID + 1, 1),
            (1, 0),
            (1, Registry::MAX_UPLOAD_BYTES + 1),
        ] {
            assert_eq!(registry.begin(1, id, 7, size), Err(Error::Invalid));
        }
        assert!(registry.high_water.is_empty());
        let initial = registry.begin(1, 1, 7, 4).unwrap();
        for (offset, bytes) in [
            (0, vec![]),
            (0, vec![0; Registry::MAX_CHUNK_BYTES + 1]),
            (4, vec![1]),
            (usize::MAX, vec![1]),
        ] {
            assert_eq!(
                registry.append(1, 1, 7, offset, &bytes),
                Err(Error::Invalid)
            );
            assert_eq!(registry.status(1, 1).unwrap(), initial);
            assert_eq!(registry.reserved, 4);
        }
    }

    #[test]
    fn active_and_reserved_limits_refuse_without_consuming_ids() {
        let mut registry = Registry::new();
        registry.begin(1, 1, 7, Registry::MAX_UPLOAD_BYTES).unwrap();
        registry.begin(1, 2, 7, Registry::MAX_UPLOAD_BYTES).unwrap();
        assert_eq!(registry.begin(1, 3, 7, 1), Err(Error::Limit));
        assert_eq!(registry.high_water[&1], 2);
        for owner in 2..=4 {
            for id in 1..=2 {
                registry
                    .begin(owner, id, 7, Registry::MAX_UPLOAD_BYTES)
                    .unwrap();
            }
        }
        assert_eq!(registry.reserved, Registry::MAX_RESERVED_BYTES);
        assert_eq!(registry.active.len(), Registry::MAX_ACTIVE);
        assert_eq!(registry.begin(5, 1, 7, 1), Err(Error::Limit));
        assert!(!registry.high_water.contains_key(&5));
        registry.discard(1, 1).unwrap();
        assert!(registry.begin(1, 3, 7, Registry::MAX_UPLOAD_BYTES).is_ok());
        assert_eq!(registry.reserved, Registry::MAX_RESERVED_BYTES);
    }

    #[test]
    fn owner_high_water_history_has_a_lifetime_limit() {
        let mut registry = Registry::new();
        for owner in 0..Registry::MAX_OWNERS as u64 {
            registry.begin(owner, 1, 7, 1).unwrap();
            registry.discard(owner, 1).unwrap();
        }
        assert_eq!(registry.begin(99, 1, 7, 1), Err(Error::Limit));
        assert!(!registry.high_water.contains_key(&99));
        assert_eq!(registry.reserved, 0);
        registry.retain_generation(8);
        assert_eq!(registry.high_water.len(), Registry::MAX_OWNERS);
        registry.remove_owner(0);
        assert!(registry.begin(99, 1, 8, 1).is_ok());
    }
}
