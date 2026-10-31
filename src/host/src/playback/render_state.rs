//! The render files in the application data directory (round record
//! 2026-10-render-modes §3.4, §3.5):
//!
//! - `render.json`: the user's preference (`auto`, `gpu`, `software`),
//!   written at once and read at the next start.
//! - `render-state.json`: the crash guard. Before GPU rendering starts the
//!   host writes `attempting`; once it ran for a few seconds, fell back on
//!   its own, or the player quit, `ok`. A start that finds `attempting` knows
//!   the last run died while GPU rendering started (a crash, a hang that was
//!   killed, a driver reset) and records `crashed`; GPU rendering then stays
//!   off until the graphics driver, the adapter or the player changes, or
//!   for [`GUARD_DAYS`] days, as Firefox's `DriverCrashGuard` does. A lost
//!   GPU device (a driver reset while the windows draw with it) is recorded
//!   too: [`LOSSES`] within a day count as a crash (record §3.5 item 4).
//!
//! Without a path (headless runs) nothing is read or written.
#![forbid(unsafe_code)]

use crate::playback::render_mode::Preference;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How long a crash keeps GPU rendering off when nothing else changes.
pub const GUARD_DAYS: u64 = 7;
/// Lost GPU devices within a day that turn GPU rendering off.
pub const LOSSES: usize = 2;
const DAY: u64 = 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardState {
    #[default]
    Ok,
    Attempting,
    Crashed,
}

/// What the guard is keyed on: a change of any of them tries GPU rendering
/// again.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardKey {
    /// The adapter's name (empty where unknown).
    pub adapter: String,
    /// The graphics driver's version; the system build where the driver
    /// comes with the system (macOS).
    pub driver: String,
    /// The player's version.
    pub app: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guard {
    #[serde(default)]
    pub state: GuardState,
    #[serde(default, flatten)]
    pub key: GuardKey,
    /// Seconds since the Unix epoch: when it was armed or found crashed.
    #[serde(default)]
    pub at: u64,
    /// When the GPU device was lost (Unix seconds), for this key, within
    /// the last day.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub losses: Vec<u64>,
}

impl Guard {
    /// Until when (Unix seconds) GPU rendering stays off for `key`, if it does.
    pub fn blocks(&self, key: &GuardKey, now: u64) -> Option<u64> {
        let until = self.at.saturating_add(GUARD_DAYS * DAY);
        (self.state == GuardState::Crashed && self.key == *key && now < until).then_some(until)
    }
}

/// `render-state.json`.
#[derive(Clone, Debug, Default)]
pub struct GuardFile {
    path: Option<PathBuf>,
}

impl GuardFile {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
    pub fn read(&self) -> Guard {
        self.path
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    fn write(&self, guard: &Guard) {
        let Some(path) = &self.path else {
            return;
        };
        let text = serde_json::to_vec_pretty(guard).unwrap_or_default();
        if let Err(error) = write_replacing(path, &text) {
            diag!("render: {} cannot be written ({error})", path.display());
        }
    }
    /// At start: an `attempting` left by the last run becomes `crashed`
    /// (now). Returns the guard as it stands.
    pub fn recover(&self, now: u64) -> Guard {
        let mut guard = self.read();
        if guard.state == GuardState::Attempting {
            guard.state = GuardState::Crashed;
            guard.at = now;
            diag!(
                "render: the last run ended while GPU rendering started ({} {}); software rendering until the driver or the player changes, or for {GUARD_DAYS} days",
                guard.key.adapter,
                guard.key.driver
            );
            self.write(&guard);
        }
        guard
    }
    /// Before GPU rendering starts. Keep the returned [`Armed`] until the
    /// player ends: dropping it settles the guard, so a start that returns
    /// with an ordinary error (or unwinds) is not taken for a crash; only a
    /// process that dies leaves `attempting` behind.
    #[must_use = "dropping it settles the guard at once"]
    pub fn arm(&self, key: GuardKey, now: u64) -> Armed {
        // Earlier device losses count while the key stays the same.
        let previous = self.read();
        let losses = if previous.key == key {
            previous.losses
        } else {
            Vec::new()
        };
        self.write(&Guard {
            state: GuardState::Attempting,
            key,
            at: now,
            losses,
        });
        Armed(self.clone())
    }
    /// The GPU device was lost (the player restarts): record it; the
    /// [`LOSSES`]th loss within a day for the same key turns the guard to
    /// `crashed`. Returns whether it did.
    pub fn device_lost(&self, key: GuardKey, now: u64) -> bool {
        let mut guard = self.read();
        if guard.key != key {
            guard = Guard {
                key,
                ..Guard::default()
            };
        }
        guard
            .losses
            .retain(|&at| at <= now && now.saturating_sub(at) < DAY);
        guard.losses.push(now);
        let crashed = guard.losses.len() >= LOSSES;
        if crashed {
            guard.state = GuardState::Crashed;
            guard.at = now;
        } else if guard.state == GuardState::Attempting {
            // The restarted player must not take this run for a crash.
            guard.state = GuardState::Ok;
        }
        self.write(&guard);
        crashed
    }
    /// GPU rendering started (or gave up on its own, or the player quits):
    /// an `attempting` becomes `ok`; anything else stays.
    pub fn settle(&self) {
        let mut guard = self.read();
        if guard.state == GuardState::Attempting {
            guard.state = GuardState::Ok;
            self.write(&guard);
        }
    }
}

/// An armed crash guard (`GuardFile::arm`); settles it when dropped.
pub struct Armed(GuardFile);

impl Drop for Armed {
    fn drop(&mut self) {
        self.0.settle();
    }
}

#[derive(Serialize, Deserialize)]
struct PreferenceFile {
    preference: String,
}

/// `render.json`: the preference, `auto` when the file is missing; a file
/// that cannot be read gives `auto` and the reason.
pub fn read_preference(path: Option<&Path>) -> (Preference, Option<String>) {
    let Some(path) = path else {
        return (Preference::Auto, None);
    };
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Preference::Auto, None);
        }
        Err(error) => return (Preference::Auto, Some(error.to_string())),
    };
    match serde_json::from_slice::<PreferenceFile>(&bytes) {
        Ok(file) => match Preference::parse(&file.preference) {
            Some(preference) => (preference, None),
            None => (
                Preference::Auto,
                Some(format!("unknown preference {:?}", file.preference)),
            ),
        },
        Err(error) => (Preference::Auto, Some(error.to_string())),
    }
}

/// Write `render.json` (replacing it whole). Nothing without a path.
pub fn write_preference(path: Option<&Path>, preference: Preference) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    let file = PreferenceFile {
        preference: preference.name().into(),
    };
    let text = serde_json::to_vec_pretty(&file).map_err(|error| error.to_string())?;
    write_replacing(path, &text).map_err(|error| error.to_string())
}

/// Written beside and renamed over `path`: a reader sees the old file or
/// the new one, never half of one.
fn write_replacing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let directory = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(directory)?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    std::io::Write::write_all(&mut temporary, bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(driver: &str) -> GuardKey {
        GuardKey {
            adapter: "AMD Radeon 780M".into(),
            driver: driver.into(),
            app: "0.5.13".into(),
        }
    }

    #[test]
    fn a_run_that_died_while_starting_blocks_until_something_changes_or_it_expires() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        assert_eq!(file.recover(100), Guard::default(), "no file: nothing");
        // The process dies: nothing drops the guard.
        std::mem::forget(file.arm(key("31.0.14005.21001"), 100));
        // The run died: the next start finds `attempting`.
        let guard = file.recover(200);
        assert_eq!((guard.state, guard.at), (GuardState::Crashed, 200));
        assert_eq!(file.read(), guard, "recorded");
        let until = 200 + GUARD_DAYS * DAY;
        assert_eq!(guard.blocks(&key("31.0.14005.21001"), 300), Some(until));
        assert_eq!(
            guard.blocks(&key("32.0.32015.7029"), 300),
            None,
            "driver updated"
        );
        let mut player = key("31.0.14005.21001");
        player.app = "0.5.14".into();
        assert_eq!(guard.blocks(&player, 300), None, "player updated");
        assert_eq!(
            guard.blocks(&key("31.0.14005.21001"), until),
            None,
            "expired"
        );
        // A second recover does not move the date.
        assert_eq!(file.recover(900).at, 200);
    }

    #[test]
    fn a_run_that_started_or_quit_settles_and_settling_keeps_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let armed = file.arm(key("1"), 100);
        file.settle();
        assert_eq!(file.read().state, GuardState::Ok);
        drop(armed);
        assert_eq!(file.recover(200).state, GuardState::Ok);
        // A start that returns early with an error drops the guard: settled.
        let start = || -> Result<(), String> {
            let _armed = file.arm(key("1"), 250);
            assert_eq!(file.read().state, GuardState::Attempting);
            Err("Backend::start failed".into())
        };
        assert!(start().is_err());
        assert_eq!(file.read().state, GuardState::Ok, "an error is not a crash");
        std::mem::forget(file.arm(key("1"), 300));
        file.recover(400);
        file.settle();
        assert_eq!(file.read().state, GuardState::Crashed);
        let none = GuardFile::new(None);
        drop(none.arm(key("1"), 1));
        assert_eq!(none.recover(2), Guard::default());
    }

    /// Branch §1.1's restart after a lost device, now through the guard:
    /// the second loss within a day turns GPU rendering off.
    #[test]
    fn a_second_device_loss_within_a_day_blocks_like_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let armed = file.arm(key("1"), 100);
        assert!(!file.device_lost(key("1"), 110), "one loss: restart on GPU");
        assert_eq!(file.read().state, GuardState::Ok, "not taken for a crash");
        std::mem::forget(armed);
        // The restarted player arms again: the loss is kept.
        std::mem::forget(file.arm(key("1"), 120));
        assert_eq!(file.read().losses, [110]);
        assert!(file.device_lost(key("1"), 130), "the second one");
        let guard = file.recover(140);
        assert_eq!(guard.state, GuardState::Crashed);
        assert_eq!(guard.blocks(&key("1"), 140), Some(130 + GUARD_DAYS * DAY));
        assert_eq!(guard.blocks(&key("2"), 140), None, "driver updated");
        // A day apart, or another driver: one loss each.
        let other = GuardFile::new(Some(dir.path().join("other.json")));
        assert!(!other.device_lost(key("1"), 100));
        assert!(!other.device_lost(key("1"), 100 + DAY));
        assert!(!other.device_lost(key("2"), 100 + DAY + 1));
        assert_eq!(other.read().losses, [100 + DAY + 1]);
    }

    #[test]
    fn the_preference_is_written_whole_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("render.json");
        assert_eq!(read_preference(Some(&path)), (Preference::Auto, None));
        write_preference(Some(&path), Preference::Software).unwrap();
        assert_eq!(read_preference(Some(&path)), (Preference::Software, None));
        write_preference(Some(&path), Preference::Gpu).unwrap();
        assert_eq!(read_preference(Some(&path)).0, Preference::Gpu);
        std::fs::write(&path, br#"{"preference":"fast"}"#).unwrap();
        let (preference, warning) = read_preference(Some(&path));
        assert_eq!(preference, Preference::Auto);
        assert!(warning.unwrap().contains("fast"));
        std::fs::write(&path, b"{").unwrap();
        assert!(read_preference(Some(&path)).1.is_some());
        assert_eq!(read_preference(None), (Preference::Auto, None));
        assert!(write_preference(None, Preference::Gpu).is_ok());
    }
}
