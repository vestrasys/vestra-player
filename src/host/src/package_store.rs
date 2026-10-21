//! Single-writer package library, not an activation engine.
//! Host owns the sole Store for this root. Journal and installed state share
//! one atomic catalog replacement; recovery never implies a runtime is ready.
#![forbid(unsafe_code)]

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{Read, Write},
    path::PathBuf,
};

use crate::package_stage::{Candidate, PackageManifest};

const MAX_SAFE: u64 = (1_u64 << 53) - 1;
const MAX_CATALOG: usize = 2 * 1024 * 1024;
const MAX_PACKAGES: usize = 128;
const MAX_OPERATIONS: usize = 512;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoredPackage {
    pub digest: String,
    pub manifest: PackageManifest,
    pub directory: PathBuf,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Installed {
    pub revision: u64,
    pub current: Option<StoredPackage>,
    pub previous: Option<StoredPackage>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Recovery {
    pub id: String,
    pub operation_id: String,
    pub before: Installed,
    pub target: StoredPackage,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Catalog {
    installed: BTreeMap<String, Installed>,
    pending: BTreeMap<String, Recovery>,
    operations: BTreeMap<String, Value>,
    // One successful commit receipt per package makes a lost commit reply
    // retry harmless; full request deduplication belongs to the manager.
    last_commit: BTreeMap<String, String>,
    /// Installed packages the user disabled (R-H10): not started at launch.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    disabled: BTreeSet<String>,
}

pub struct Store {
    root: PathBuf,
    catalog: Catalog,
}

fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b)),
        "invalid package or operation identity"
    );
    Ok(())
}

impl Store {
    pub fn open(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(&root)?;
        ensure!(
            !fs::symlink_metadata(&root)?.file_type().is_symlink(),
            "store root must not be a symlink"
        );
        let root = root.canonicalize()?;
        fs::create_dir_all(root.join("packages"))?;
        ensure!(
            !fs::symlink_metadata(root.join("packages"))?
                .file_type()
                .is_symlink(),
            "package library must not be a symlink"
        );
        let path = root.join("catalog.json");
        let catalog = if path.exists() {
            let metadata = fs::symlink_metadata(&path)?;
            ensure!(
                metadata.is_file()
                    && !metadata.file_type().is_symlink()
                    && metadata.len() <= MAX_CATALOG as u64,
                "invalid package catalog"
            );
            let mut bytes = Vec::new();
            File::open(path)?
                .take(MAX_CATALOG as u64 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= MAX_CATALOG, "package catalog too large");
            serde_json::from_slice(&bytes).context("parse package catalog")?
        } else {
            Catalog::default()
        };
        let store = Self { root, catalog };
        store.validate_catalog()?;
        Ok(store)
    }

    fn validate_package(&self, package: &StoredPackage) -> Result<()> {
        identifier(&package.manifest.id)?;
        ensure!(
            package.digest.len() == 64
                && package
                    .digest
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid package digest"
        );
        ensure!(
            package.directory == self.root.join("packages").join(&package.digest),
            "package directory is outside this library"
        );
        Ok(())
    }

    fn verify_package(&self, package: &StoredPackage) -> Result<()> {
        self.validate_package(package)?;
        let captured = Candidate::capture(&package.directory)?;
        ensure!(
            captured.digest() == package.digest
                && serde_json::to_value(captured.manifest())?
                    == serde_json::to_value(&package.manifest)?,
            "stored package bytes or manifest changed"
        );
        Ok(())
    }

    fn validate_catalog(&self) -> Result<()> {
        ensure!(
            self.catalog.installed.len() <= MAX_PACKAGES
                && self.catalog.pending.len() <= 64
                && self.catalog.operations.len() <= MAX_OPERATIONS
                && self.catalog.last_commit.len() <= MAX_PACKAGES,
            "package catalog count exceeded"
        );
        for (id, installed) in &self.catalog.installed {
            identifier(id)?;
            ensure!(
                installed.revision <= MAX_SAFE,
                "installed revision exhausted"
            );
            for package in [&installed.current, &installed.previous]
                .into_iter()
                .flatten()
            {
                self.validate_package(package)?;
                ensure!(
                    package.manifest.id == *id,
                    "installed package identity mismatch"
                );
            }
        }
        for (id, recovery) in &self.catalog.pending {
            identifier(id)?;
            identifier(&recovery.operation_id)?;
            ensure!(
                recovery.id == *id && recovery.target.manifest.id == *id,
                "recovery identity mismatch"
            );
            self.validate_package(&recovery.target)?;
            ensure!(
                serde_json::to_value(&recovery.before)? == serde_json::to_value(self.get(id))?,
                "recovery baseline mismatch"
            );
        }
        for id in self.catalog.operations.keys() {
            identifier(id)?;
        }
        for (id, operation) in &self.catalog.last_commit {
            identifier(id)?;
            identifier(operation)?;
        }
        Ok(())
    }

    fn publish(&mut self, next: Catalog) -> Result<()> {
        let bytes = serde_json::to_vec(&next)?;
        ensure!(
            bytes
                .len()
                .checked_add(next.pending.len() * 64 * 1024)
                .is_some_and(|size| size <= MAX_CATALOG),
            "package catalog byte limit or reserved terminal receipt capacity exceeded"
        );
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(self.root.join("catalog.json"))
            .map_err(|error| error.error)?;
        // Publish succeeded even if the directory durability barrier fails:
        // callers must query/recover rather than blindly repeat an operation.
        self.catalog = next;
        File::open(&self.root)?.sync_all()?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Installed {
        self.catalog.installed.get(id).cloned().unwrap_or_default()
    }

    pub fn installed(&self) -> BTreeMap<String, Installed> {
        self.catalog.installed.clone()
    }

    pub fn stage(&self, candidate: &Candidate) -> Result<StoredPackage> {
        let target = StoredPackage {
            digest: candidate.digest().into(),
            manifest: candidate.manifest().clone(),
            directory: self.root.join("packages").join(candidate.digest()),
        };
        self.validate_package(&target)?;
        if target.directory.exists() {
            self.verify_package(&target)?;
            return Ok(target);
        }
        // Capture into a fresh bounded private tree before copying. The public
        // Candidate path is not an OS-immutable filesystem object.
        let source = Candidate::capture(candidate.directory())?;
        ensure!(
            source.digest() == candidate.digest(),
            "candidate bytes changed"
        );
        let temporary = tempfile::Builder::new()
            .prefix(".stage-")
            .tempdir_in(self.root.join("packages"))?;
        copy_tree(source.directory(), temporary.path())?;
        let verified = Candidate::capture(temporary.path())?;
        ensure!(
            verified.digest() == candidate.digest(),
            "candidate changed during staging"
        );
        fs::rename(temporary.path(), &target.directory)?;
        File::open(self.root.join("packages"))?.sync_all()?;
        Ok(target)
    }

    pub fn prepare(
        &mut self,
        id: &str,
        expected_revision: u64,
        target: &StoredPackage,
        operation_id: &str,
    ) -> Result<()> {
        self.prepare_with_receipt(
            id,
            expected_revision,
            target,
            operation_id,
            serde_json::json!({"operationId":operation_id,"packageId":id,"status":"preparing"}),
        )
    }

    pub fn prepare_with_receipt(
        &mut self,
        id: &str,
        expected_revision: u64,
        target: &StoredPackage,
        operation_id: &str,
        receipt: Value,
    ) -> Result<()> {
        identifier(id)?;
        identifier(operation_id)?;
        check_receipt(&receipt)?;
        ensure!(
            expected_revision < MAX_SAFE,
            "invalid or exhausted revision"
        );
        ensure!(target.manifest.id == id, "target identity mismatch");
        self.verify_package(target)?;
        ensure!(
            !self.catalog.pending.contains_key(id),
            "package transaction already pending"
        );
        ensure!(
            self.catalog.pending.len() < 64,
            "pending package limit reached"
        );
        ensure!(
            self.catalog.installed.contains_key(id)
                || self.catalog.installed.len()
                    + self
                        .catalog
                        .pending
                        .keys()
                        .filter(|key| !self.catalog.installed.contains_key(*key))
                        .count()
                    < MAX_PACKAGES,
            "installed package limit reached"
        );
        let before = self.get(id);
        ensure!(
            before.revision == expected_revision,
            "installed revision conflict"
        );
        let mut next = self.catalog.clone();
        next.pending.insert(
            id.into(),
            Recovery {
                id: id.into(),
                operation_id: operation_id.into(),
                before,
                target: target.clone(),
            },
        );
        insert_receipt(&mut next, operation_id, receipt)?;
        self.publish(next)
    }

    pub fn commit(&mut self, id: &str, operation_id: &str) -> Result<Installed> {
        self.commit_with_receipt(
            id,
            operation_id,
            serde_json::json!({"operationId":operation_id,"packageId":id,"status":"committed"}),
        )
    }

    pub fn commit_with_receipt(
        &mut self,
        id: &str,
        operation_id: &str,
        mut receipt: Value,
    ) -> Result<Installed> {
        identifier(id)?;
        identifier(operation_id)?;
        if self
            .catalog
            .last_commit
            .get(id)
            .is_some_and(|previous| previous == operation_id)
        {
            return Ok(self.get(id));
        }
        let recovery = self
            .catalog
            .pending
            .get(id)
            .context("package transaction missing")?
            .clone();
        ensure!(
            recovery.operation_id == operation_id,
            "package operation conflict"
        );
        self.verify_package(&recovery.target)?;
        let revision = recovery
            .before
            .revision
            .checked_add(1)
            .filter(|n| *n <= MAX_SAFE)
            .context("installed revision exhausted")?;
        let installed = Installed {
            revision,
            current: Some(recovery.target),
            previous: recovery.before.current,
        };
        ensure!(receipt.is_object(), "operation receipt must be an object");
        receipt["installed"] = serde_json::to_value(&installed)?;
        check_receipt(&receipt)?;
        let mut next = self.catalog.clone();
        next.pending.remove(id);
        insert_receipt(&mut next, operation_id, receipt)?;
        next.installed.insert(id.into(), installed.clone());
        next.last_commit.insert(id.into(), operation_id.into());
        // Installing or rolling back is asking for the package to run.
        next.disabled.remove(id);
        self.publish(next)?;
        Ok(installed)
    }

    pub fn abort(&mut self, id: &str, operation_id: &str) -> Result<()> {
        self.abort_with_receipt(
            id,
            operation_id,
            serde_json::json!({"operationId":operation_id,"packageId":id,"status":"aborted"}),
        )
    }

    pub fn abort_with_receipt(
        &mut self,
        id: &str,
        operation_id: &str,
        receipt: Value,
    ) -> Result<()> {
        identifier(id)?;
        check_receipt(&receipt)?;
        identifier(operation_id)?;
        let pending = self
            .catalog
            .pending
            .get(id)
            .context("package transaction missing")?;
        ensure!(
            pending.operation_id == operation_id,
            "package operation conflict"
        );
        let mut next = self.catalog.clone();
        next.pending.remove(id);
        insert_receipt(&mut next, operation_id, receipt)?;
        self.publish(next)
    }

    pub fn pending(&self) -> Vec<Recovery> {
        self.catalog.pending.values().cloned().collect()
    }

    pub fn save_operation(&mut self, id: &str, value: &Value) -> Result<()> {
        identifier(id)?;
        check_receipt(value)?;
        let mut next = self.catalog.clone();
        insert_receipt(&mut next, id, value.clone())?;
        self.publish(next)
    }

    /// Only call before staging a new target, while no temporary unjournaled
    /// StoredPackage is in use. This is single-writer library garbage collection.
    pub fn gc(&self) -> Result<()> {
        let mut referenced = BTreeSet::new();
        for installed in self.catalog.installed.values() {
            for package in [&installed.current, &installed.previous]
                .into_iter()
                .flatten()
            {
                referenced.insert(package.digest.clone());
            }
        }
        for recovery in self.catalog.pending.values() {
            referenced.insert(recovery.target.digest.clone());
        }
        let packages = self.root.join("packages");
        for entry in fs::read_dir(&packages)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let digest_name = name.len() == 64
                && name
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !digest_name && !name.starts_with(".stage-") {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())?;
            ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "unexpected package library entry"
            );
            if !referenced.contains(name) {
                fs::remove_dir_all(entry.path())?;
            }
        }
        File::open(packages)?.sync_all()?;
        Ok(())
    }

    pub fn operations(&self) -> BTreeMap<String, Value> {
        self.catalog.operations.clone()
    }

    pub fn disabled(&self) -> &BTreeSet<String> {
        &self.catalog.disabled
    }

    /// Record whether an installed package stays stopped across launches.
    pub fn set_disabled(&mut self, id: &str, disabled: bool) -> Result<()> {
        identifier(id)?;
        ensure!(
            !disabled || self.catalog.installed.contains_key(id),
            "only installed packages can be disabled"
        );
        if self.catalog.disabled.contains(id) == disabled {
            return Ok(());
        }
        let mut next = self.catalog.clone();
        if disabled {
            next.disabled.insert(id.into());
        } else {
            next.disabled.remove(id);
        }
        self.publish(next)
    }

    /// Remove an installed package (current and previous) in one catalog
    /// replacement with its receipt, then collect its unreferenced files.
    pub fn uninstall(&mut self, id: &str, operation_id: &str, receipt: Value) -> Result<()> {
        identifier(id)?;
        identifier(operation_id)?;
        check_receipt(&receipt)?;
        ensure!(
            self.catalog.installed.contains_key(id),
            "package is not installed"
        );
        ensure!(
            !self.catalog.pending.contains_key(id),
            "package transaction already pending"
        );
        let mut next = self.catalog.clone();
        next.installed.remove(id);
        next.disabled.remove(id);
        next.last_commit.remove(id);
        insert_receipt(&mut next, operation_id, receipt)?;
        self.publish(next)?;
        self.gc()
    }
}

fn check_receipt(value: &Value) -> Result<()> {
    ensure!(
        value.is_object() && serde_json::to_vec(value)?.len() <= 64 * 1024,
        "operation receipt must be an object within 64KiB"
    );
    Ok(())
}

fn insert_receipt(catalog: &mut Catalog, id: &str, value: Value) -> Result<()> {
    ensure!(
        catalog.operations.contains_key(id) || catalog.operations.len() < MAX_OPERATIONS,
        "operation history capacity reached; existing operations remain queryable"
    );
    catalog.operations.insert(id.into(), value);
    Ok(())
}

fn copy_tree(source: &std::path::Path, destination: &std::path::Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "staged symlink rejected"
        );
        let target = destination.join(entry.file_name());
        if metadata.is_dir() {
            fs::create_dir(&target)?;
            copy_tree(&entry.path(), &target)?;
        } else {
            ensure!(metadata.is_file(), "staged special file rejected");
            let mut input = File::open(entry.path())?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(target)?;
            let copied = std::io::copy(&mut (&mut input).take(32 * 1024 * 1024 + 1), &mut output)?;
            ensure!(copied <= 32 * 1024 * 1024, "staged file size exceeded");
            output.sync_all()?;
        }
    }
    File::open(destination)?.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn candidate(version: &str) -> (tempfile::TempDir, Candidate) {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("manifest.json"), serde_json::to_vec(&serde_json::json!({"id":"counter","apiVersion":crate::plugins::API,"version":version,"entry":"main.js","capabilities":[]})).unwrap()).unwrap();
        fs::write(
            root.path().join("main.js"),
            format!("export const version = '{version}';"),
        )
        .unwrap();
        let candidate = Candidate::capture(root.path()).unwrap();
        (root, candidate)
    }

    #[test]
    fn pending_survives_reopen_and_abort_does_not_install() {
        let root = tempfile::tempdir().unwrap();
        let (_, candidate) = candidate("1.0.0");
        let mut store = Store::open(root.path().into()).unwrap();
        let package = store.stage(&candidate).unwrap();
        store.prepare("counter", 0, &package, "first").unwrap();
        drop(store);
        let mut store = Store::open(root.path().into()).unwrap();
        assert_eq!(store.pending().len(), 1);
        assert!(store.get("counter").current.is_none());
        assert!(store.abort("counter", "wrong").is_err());
        store.abort("counter", "first").unwrap();
        assert!(store.pending().is_empty());
    }

    #[test]
    fn install_upgrade_rollback_are_atomic_revisions_and_commit_retry_is_harmless() {
        let root = tempfile::tempdir().unwrap();
        let (_, first) = candidate("1.0.0");
        let (_, second) = candidate("1.1.0");
        let mut store = Store::open(root.path().into()).unwrap();
        let first = store.stage(&first).unwrap();
        let second = store.stage(&second).unwrap();
        store.prepare("counter", 0, &first, "one").unwrap();
        assert!(store.prepare("counter", 0, &second, "other").is_err());
        assert!(store.commit("counter", "wrong").is_err());
        assert_eq!(store.commit("counter", "one").unwrap().revision, 1);
        assert_eq!(store.commit("counter", "one").unwrap().revision, 1);
        assert!(store.prepare("counter", 0, &second, "two").is_err());
        store.prepare("counter", 1, &second, "two").unwrap();
        let installed = store.commit("counter", "two").unwrap();
        assert_eq!(installed.current.unwrap().manifest.version, "1.1.0");
        let previous = installed.previous.unwrap();
        store.prepare("counter", 2, &previous, "back").unwrap();
        store.commit("counter", "back").unwrap();
        store
            .save_operation("back", &serde_json::json!({"status":"completed"}))
            .unwrap();
        drop(store);
        let store = Store::open(root.path().into()).unwrap();
        let installed = store.get("counter");
        assert_eq!(installed.revision, 3);
        assert_eq!(installed.current.unwrap().manifest.version, "1.0.0");
        assert_eq!(installed.previous.unwrap().manifest.version, "1.1.0");
        assert_eq!(store.operations()["back"]["status"], "completed");
    }

    #[test]
    fn disabled_set_persists_and_uninstall_removes_files_and_flag() {
        let root = tempfile::tempdir().unwrap();
        let (_, first) = candidate("1.0.0");
        let mut store = Store::open(root.path().into()).unwrap();
        let first = store.stage(&first).unwrap();
        assert!(
            store.set_disabled("counter", true).is_err(),
            "not installed"
        );
        store.prepare("counter", 0, &first, "one").unwrap();
        store.commit("counter", "one").unwrap();
        store.set_disabled("counter", true).unwrap();
        drop(store);
        let mut store = Store::open(root.path().into()).unwrap();
        assert!(store.disabled().contains("counter"));
        store
            .uninstall("counter", "gone", serde_json::json!({"status":"committed"}))
            .unwrap();
        assert!(store.get("counter").current.is_none());
        assert!(store.disabled().is_empty());
        assert!(!first.directory.exists(), "files collected");
        assert_eq!(store.operations()["gone"]["status"], "committed");
        assert!(
            store
                .uninstall("counter", "again", serde_json::json!({}))
                .is_err()
        );
    }

    #[test]
    fn changed_candidate_or_library_bytes_cannot_commit() {
        let root = tempfile::tempdir().unwrap();
        let (_, candidate) = candidate("1.0.0");
        let mut store = Store::open(root.path().into()).unwrap();
        let stored = store.stage(&candidate).unwrap();
        store.prepare("counter", 0, &stored, "one").unwrap();
        fs::write(stored.directory.join("main.js"), "changed").unwrap();
        assert!(store.commit("counter", "one").is_err());
        assert!(store.get("counter").current.is_none());
        assert_eq!(store.pending().len(), 1);
        fs::write(candidate.directory().join("main.js"), "changed").unwrap();
        let other = tempfile::tempdir().unwrap();
        assert!(
            Store::open(other.path().into())
                .unwrap()
                .stage(&candidate)
                .is_err()
        );
    }

    #[test]
    fn receipt_and_revision_reopen_together_and_gc_keeps_recovery_references() {
        let root = tempfile::tempdir().unwrap();
        let (_, first) = candidate("1.0.0");
        let (_, second) = candidate("1.1.0");
        let (_, unused) = candidate("1.2.0");
        let mut store = Store::open(root.path().into()).unwrap();
        let first = store.stage(&first).unwrap();
        let second = store.stage(&second).unwrap();
        let unused = store.stage(&unused).unwrap();
        store
            .prepare_with_receipt(
                "counter",
                0,
                &first,
                "one",
                serde_json::json!({"status":"preparing","fingerprint":"one"}),
            )
            .unwrap();
        store
            .commit_with_receipt(
                "counter",
                "one",
                serde_json::json!({"status":"committed","fingerprint":"one"}),
            )
            .unwrap();
        store
            .prepare_with_receipt(
                "counter",
                1,
                &second,
                "two",
                serde_json::json!({"status":"preparing","fingerprint":"two"}),
            )
            .unwrap();
        store.gc().unwrap();
        assert!(first.directory.exists());
        assert!(second.directory.exists());
        assert!(!unused.directory.exists());
        drop(store);
        let mut store = Store::open(root.path().into()).unwrap();
        assert_eq!(store.operations()["one"]["installed"]["revision"], 1);
        assert_eq!(store.operations()["one"]["fingerprint"], "one");
        assert_eq!(store.operations()["two"]["status"], "preparing");
        assert_eq!(store.pending().len(), 1);
        store
            .abort_with_receipt(
                "counter",
                "two",
                serde_json::json!({"status":"failed","restored":true}),
            )
            .unwrap();
        drop(store);
        let store = Store::open(root.path().into()).unwrap();
        assert!(store.pending().is_empty());
        assert_eq!(store.operations()["two"]["status"], "failed");
        assert_eq!(store.installed()["counter"].revision, 1);
        store.gc().unwrap();
        assert!(!second.directory.exists());
    }

    #[test]
    fn full_receipt_history_rejects_new_requests_and_preserves_existing() {
        let root = tempfile::tempdir().unwrap();
        let (_, candidate) = candidate("1.0.0");
        let mut store = Store::open(root.path().into()).unwrap();
        let package = store.stage(&candidate).unwrap();
        store.prepare("counter", 0, &package, "a-pending").unwrap();
        // Populate directly to exercise the bounded insertion without 512
        // redundant disk durability barriers in this unit test.
        for index in 0..511 {
            store.catalog.operations.insert(
                format!("old-{index:03}"),
                serde_json::json!({"status":"completed"}),
            );
        }
        assert!(
            store
                .save_operation("new", &serde_json::json!({"status":"completed"}))
                .is_err()
        );
        assert_eq!(store.operations().len(), 512);
        assert!(store.operations().contains_key("a-pending"));
        assert!(store.operations().contains_key("old-000"));
        assert!(!store.operations().contains_key("new"));
        assert!(
            store
                .commit_with_receipt(
                    "counter",
                    "a-pending",
                    serde_json::json!({"status":"committed","message":"x".repeat(64*1024)})
                )
                .is_err()
        );
        assert_eq!(store.get("counter").revision, 0);
        assert_eq!(store.pending().len(), 1);
    }
}
