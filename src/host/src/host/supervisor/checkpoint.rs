//! Checkpoints (doc/architecture/supervisor.md §2.2; MOVE of `lazy.rs`'s
//! recordings, B2c): what a unit in standby registered, so the next start
//! replays it instead of starting the plugin (round record
//! doc/process/rounds/2026-10-lazy-activation.md §2.1). The core decides
//! when one is written (`Action::Checkpoint`) and what it says (resident or
//! not); the adapter writes and reads the file.
use super::probe;
use crate::capabilities::Call;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The recording format.
pub(crate) const SCHEMA: u32 = 1;

/// What a recording is valid for (§2.1): the installed instance, the
/// package digest and version; the player version and build are checked
/// too (a new player records again, the safe side).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Key {
    pub guid: String,
    pub digest: String,
    pub version: Option<String>,
}

/// `<package store>/activation/<instance GUID>.json`.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Recording {
    pub schema: u32,
    pub player: String,
    #[serde(default)]
    pub build: Option<String>,
    pub digest: String,
    #[serde(default)]
    pub version: Option<String>,
    /// Whether it held a live resource at its last `runtime.ready` (§3.4):
    /// the next start launches it instead of replaying `calls`.
    pub resident_at_ready: bool,
    pub recorded_at: u64,
    pub calls: Vec<Call>,
    /// Written under `VESTRA_TEST_PARK=all` (`probe.rs`): a resident plugin
    /// recorded as lazy, without what it holds live. Only a run with the
    /// same switch reads it back; any other run ignores it as stale.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub test_park: bool,
}
impl Recording {
    fn new(key: &Key, calls: Vec<Call>, resident_at_ready: bool) -> Self {
        let recorded_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        Self {
            schema: SCHEMA,
            player: env!("CARGO_PKG_VERSION").into(),
            build: option_env!("VESTRA_BUILD").map(str::to_owned),
            digest: key.digest.clone(),
            version: key.version.clone(),
            resident_at_ready,
            recorded_at,
            calls,
            test_park: probe::park_all(),
        }
    }
    /// Valid for this package and this player.
    fn matches(&self, key: &Key) -> bool {
        self.schema == SCHEMA
            && self.player == env!("CARGO_PKG_VERSION")
            && self.build.as_deref() == option_env!("VESTRA_BUILD")
            && self.digest == key.digest
            && self.version == key.version
            && self.test_park == probe::park_all()
    }
}

fn file(directory: &Path, guid: &str) -> PathBuf {
    directory.join(format!("{guid}.json"))
}

/// The recording of `key`'s instance, if there is a valid one; a stale or
/// corrupt one is ignored (the plugin starts as before, §2.2).
pub(crate) fn read(directory: &Path, key: &Key) -> Option<Recording> {
    let path = file(directory, &key.guid);
    let bytes = std::fs::read(&path).ok()?;
    match serde_json::from_slice::<Recording>(&bytes) {
        Ok(recording) if recording.matches(key) => Some(recording),
        Ok(_) => {
            diag!("lazy: {} is of another package or player", path.display());
            None
        }
        Err(error) => {
            diag!("lazy: {}: {error}", path.display());
            None
        }
    }
}

/// Writes `key`'s recording (a temporary file renamed over the old one).
pub(crate) fn write(
    directory: &Path,
    key: &Key,
    calls: Vec<Call>,
    resident_at_ready: bool,
) -> Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = file(directory, &key.guid);
    let temporary = path.with_extension("json.tmp");
    let recording = Recording::new(key, calls, resident_at_ready);
    std::fs::write(&temporary, serde_json::to_vec(&recording)?)?;
    std::fs::rename(&temporary, &path)?;
    Ok(())
}

/// Removes `guid`'s recording (uninstall, reload, upgrade).
pub(crate) fn remove(directory: &Path, guid: &str) {
    let path = file(directory, guid);
    if let Err(error) = std::fs::remove_file(&path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        diag!("lazy: {}: {error}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn key() -> Key {
        Key {
            guid: "3f0c8a8e-5b7e-4c1f-9d0a-2b6f1c7e9a41".into(),
            digest: "abc".into(),
            version: Some("1.0.0".into()),
        }
    }

    #[test]
    fn a_recording_is_read_back_only_for_its_package_and_player() {
        let directory = tempfile::tempdir().unwrap();
        let calls = vec![Call::new(
            "commands.register",
            json!({"id":"hello","revision":1,"declaration":{"title":"Hello"},"state":{"enabled":true}}),
        )];
        write(directory.path(), &key(), calls.clone(), false).unwrap();
        let read_back = read(directory.path(), &key()).unwrap();
        assert_eq!(read_back.calls, calls);
        assert!(!read_back.resident_at_ready);
        // Resident at its ready: read back as such (the start launches it).
        write(directory.path(), &key(), Vec::new(), true).unwrap();
        let resident = read(directory.path(), &key()).unwrap();
        assert!(resident.resident_at_ready && resident.calls.is_empty());
        write(directory.path(), &key(), calls.clone(), false).unwrap();
        // Another digest, version or instance: not this package's.
        for other in [
            Key {
                digest: "other".into(),
                ..key()
            },
            Key {
                version: Some("1.0.1".into()),
                ..key()
            },
        ] {
            assert!(read(directory.path(), &other).is_none());
        }
        let path = file(directory.path(), &key().guid);
        let mut json: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        // A normal run writes no test mark and ignores a recording made
        // under `VESTRA_TEST_PARK=all` (incomplete for a resident plugin).
        assert!(json.get("testPark").is_none(), "{json}");
        json["testPark"] = true.into();
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(read(directory.path(), &key()).is_none());
        json["testPark"] = false.into();
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(read(directory.path(), &key()).is_some());
        // Another player release records again.
        json["player"] = "0.0.1".into();
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(read(directory.path(), &key()).is_none());
        // Corrupt: ignored.
        std::fs::write(&path, "{").unwrap();
        assert!(read(directory.path(), &key()).is_none());
        remove(directory.path(), &key().guid);
        assert!(!path.exists());
        remove(directory.path(), &key().guid);
    }
}
