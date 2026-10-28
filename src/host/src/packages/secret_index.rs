//! The names each plugin service has stored in the system secret store
//! (`secrets.*`). Keychains cannot list the entries of one service on every
//! system, so the host records the names it wrote, in its own package
//! library (`<library>/secrets/<service>.json`, names only, never values;
//! plugins cannot write there). Uninstalling with `removeData`, or dropping a
//! retired instance, deletes the recorded entries.
#![forbid(unsafe_code)]
use crate::RpcError;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

/// Names per service; `secrets.set` of a new name beyond it fails `limit`.
pub const MAX_NAMES: usize = 256;

#[derive(Default)]
struct Entry {
    names: BTreeSet<String>,
    /// Bumped by every `reserve`, so a delete that finishes after a newer
    /// set of the same name does not forget it.
    generations: BTreeMap<String, u64>,
}

struct Inner {
    /// `None`: nothing persisted (no library directory).
    directory: Option<PathBuf>,
    services: BTreeMap<String, Entry>,
    next: u64,
}

/// Shared between the UI thread and the secret worker threads.
#[derive(Clone)]
pub struct Index(Arc<Mutex<Inner>>);

impl Index {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self(Arc::new(Mutex::new(Inner {
            directory,
            services: BTreeMap::new(),
            next: 0,
        })))
    }

    /// Before `secrets.set`: records `name` (written first, so a crash
    /// between the two leaves a name to delete rather than a lost entry).
    pub fn reserve(&self, service: &str, name: &str) -> Result<(), RpcError> {
        let mut inner = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        inner.next += 1;
        let generation = inner.next;
        let directory = inner.directory.clone();
        let entry = inner.entry(service);
        if !entry.names.contains(name) && entry.names.len() >= MAX_NAMES {
            return Err(RpcError::new(
                "limit",
                format!("a plugin stores at most {MAX_NAMES} secret names"),
            ));
        }
        entry.generations.insert(name.into(), generation);
        if entry.names.insert(name.into()) {
            save(directory.as_deref(), service, &entry.names);
        }
        Ok(())
    }

    /// The generation `release` compares against (taken before a delete).
    pub fn generation(&self, service: &str, name: &str) -> u64 {
        let mut inner = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        inner
            .entry(service)
            .generations
            .get(name)
            .copied()
            .unwrap_or(0)
    }

    /// After a successful `secrets.delete`: forgets `name` unless it was
    /// set again meanwhile.
    pub fn release(&self, service: &str, name: &str, generation: u64) {
        let mut inner = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let directory = inner.directory.clone();
        let entry = inner.entry(service);
        if entry.generations.get(name).copied().unwrap_or(0) != generation {
            return;
        }
        entry.generations.remove(name);
        if entry.names.remove(name) {
            save(directory.as_deref(), service, &entry.names);
        }
    }

    /// Deletes every recorded entry of `service` with `delete` and the
    /// record itself. Blocking (keychain calls, possibly a system prompt):
    /// run off the UI thread; the lock is not held meanwhile. Entries that
    /// fail to delete are logged and stay in the record.
    pub fn purge(
        &self,
        service: &str,
        delete: impl Fn(&str, &str) -> Result<(), RpcError>,
    ) -> usize {
        let names = {
            let mut inner = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            let entry = inner.entry(service);
            entry.generations.clear();
            entry.names.clone()
        };
        let mut failed = Vec::new();
        for name in &names {
            if let Err(error) = delete(service, name) {
                diag!(
                    "secrets {service}: could not delete {name}: {}",
                    error.message
                );
                failed.push(name);
            }
        }
        let mut inner = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let directory = inner.directory.clone();
        let entry = inner.entry(service);
        // Names set again meanwhile were reserved after the snapshot.
        entry.names.retain(|name| {
            !names.contains(name) || failed.contains(&name) || entry.generations.contains_key(name)
        });
        save(directory.as_deref(), service, &entry.names);
        if entry.names.is_empty() {
            inner.services.remove(service);
        }
        names.len() - failed.len()
    }

    #[cfg(test)]
    fn names(&self, service: &str) -> Vec<String> {
        let mut inner = self.0.lock().unwrap();
        inner.entry(service).names.iter().cloned().collect()
    }
}

impl Inner {
    fn entry(&mut self, service: &str) -> &mut Entry {
        let directory = self.directory.clone();
        self.services
            .entry(service.into())
            .or_insert_with(|| Entry {
                names: load(directory.as_deref(), service),
                generations: BTreeMap::new(),
            })
    }
}

fn file(directory: &Path, service: &str) -> PathBuf {
    directory.join(format!("{service}.json"))
}

fn load(directory: Option<&Path>, service: &str) -> BTreeSet<String> {
    let Some(directory) = directory else {
        return BTreeSet::new();
    };
    let path = file(directory, service);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<Vec<String>>(&bytes)
            .map(|names| names.into_iter().take(MAX_NAMES).collect())
            .unwrap_or_else(|error| {
                diag!("secrets index {}: {error}", path.display());
                BTreeSet::new()
            }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
        Err(error) => {
            diag!("secrets index {}: {error}", path.display());
            BTreeSet::new()
        }
    }
}

/// Writes the record (temporary file, then rename); an empty one is removed.
/// A failure is logged and does not fail the secret operation.
fn save(directory: Option<&Path>, service: &str, names: &BTreeSet<String>) {
    let Some(directory) = directory else {
        return;
    };
    let path = file(directory, service);
    let result = (|| -> std::io::Result<()> {
        if names.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(directory)?;
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec(names)?)?;
        std::fs::rename(&temporary, &path)
    })();
    if let Err(error) = result {
        diag!("secrets index {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_recorded_bounded_and_purged() {
        let dir = tempfile::tempdir().unwrap();
        let index = Index::new(Some(dir.path().join("secrets")));
        let service = "vestra.plugin.1";
        index.reserve(service, "token").unwrap();
        index.reserve(service, "token").unwrap();
        index.reserve(service, "user").unwrap();
        // Persisted: a fresh index reads the same names.
        let again = Index::new(Some(dir.path().join("secrets")));
        assert_eq!(again.names(service), ["token", "user"]);
        // A delete releases its name, unless the name was set again meanwhile.
        let before = index.generation(service, "user");
        index.release(service, "user", before);
        assert_eq!(index.names(service), ["token"]);
        let stale = index.generation(service, "token");
        index.reserve(service, "token").unwrap();
        index.release(service, "token", stale);
        assert_eq!(index.names(service), ["token"]);
        // At most MAX_NAMES; existing names can still be replaced.
        for n in 1..MAX_NAMES {
            index.reserve(service, &format!("n{n}")).unwrap();
        }
        assert_eq!(
            index.reserve(service, "one-more").unwrap_err().code,
            "limit"
        );
        index.reserve(service, "token").unwrap();
        // Other services are independent.
        index.reserve("vestra.plugin.2", "token").unwrap();
        // Purge: every name deleted, failures kept for later.
        let deleted = std::cell::RefCell::new(Vec::new());
        let count = index.purge(service, |s, name| {
            assert_eq!(s, service);
            if name == "n7" {
                return Err(RpcError::new("failed", "denied"));
            }
            deleted.borrow_mut().push(name.to_owned());
            Ok(())
        });
        assert_eq!(count, MAX_NAMES - 1);
        assert!(deleted.borrow().contains(&"token".to_owned()));
        assert_eq!(index.names(service), ["n7"]);
        index.purge(service, |_, _| Ok(()));
        assert!(index.names(service).is_empty());
        assert!(!dir.path().join("secrets/vestra.plugin.1.json").exists());
        assert_eq!(index.names("vestra.plugin.2"), ["token"]);
    }
}
