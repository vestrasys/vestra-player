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
//!   Plugin shaders have a record of their own in the same file
//!   ([`ShaderRecord`], `shaders`): `applying` before a non-empty plugin
//!   shader list goes to mpv, `ok` after [`SHADER_STABLE_SECS`] of playback
//!   with it, when the lists empty or the player quits. A start that finds
//!   `applying` records `crashed`, and that start applies no plugin shaders;
//!   [`SHADER_CRASHES`] such crashes within a day keep them off until the
//!   driver or the player changes, or for [`GUARD_DAYS`] days.
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

/// What a record's key names (`Guard::schema`). Below [`KEYED`] (a file
/// written before host 0.5.19, without the field): the first graphics
/// adapter, whatever `VESTRA_GPU_ADAPTER` chose. From [`KEYED`] on: the
/// adapter the mode used. [`SCHEMA`] (0.5.23) also keeps a record per key
/// (`Guard::others`).
pub const SCHEMA: u32 = 3;
/// The first schema whose key names the adapter in use.
pub const KEYED: u32 = 2;

/// `render-state.json`: the current record (the key armed or lost last) at
/// the top level, as before, and the records of other keys still worth
/// keeping in `others` (record 2026-10-render-modes §12: GPU rendering and
/// CPU mode's Direct3D 12 windows may use different adapters on a hybrid
/// laptop, and one must not overwrite the other's crash).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guard {
    /// Which adapter `key` names ([`KEYED`]); 0 for older files.
    #[serde(default)]
    pub schema: u32,
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
    /// The records of other keys (none of their own `others`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<Guard>,
    /// The plugin shaders' record (top level only; host 0.6.3). Older
    /// players ignore it (and drop it when they rewrite the file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shaders: Option<ShaderRecord>,
}

/// Seconds of playback with plugin shaders on after which they count as
/// working (`ok`).
pub const SHADER_STABLE_SECS: u64 = 10;
/// Runs that ended while plugin shaders were on, within a day, that keep
/// them off for [`GUARD_DAYS`]; fewer keep them off for one start.
pub const SHADER_CRASHES: usize = 2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShaderState {
    #[default]
    Ok,
    Applying,
    Crashed,
}

/// `render-state.json`'s `shaders`: whether the last run ended while
/// plugin shaders were on. Keyed like the GPU records (GPU rendering's
/// adapter, its driver, the player): plugin shaders run only there.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderRecord {
    #[serde(default)]
    pub state: ShaderState,
    #[serde(default, flatten)]
    pub key: GuardKey,
    /// Unix seconds: when it was armed, or found crashed.
    #[serde(default)]
    pub at: u64,
    /// The plugins whose shaders were on (manifest ids, no paths).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<String>,
    /// When runs ended while they were on (Unix seconds), within a day.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crashes: Vec<u64>,
    /// The start that found the crash kept plugin shaders off already (a
    /// single crash keeps them off for one start only).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub skipped: bool,
}

/// Why plugin shaders are off for this run (`GuardFile::shader_start`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderBlock {
    /// The plugins whose shaders were on when the run ended.
    pub plugins: Vec<String>,
    /// Until when (Unix seconds) they stay off; `None`: this start only.
    pub until: Option<u64>,
}

impl ShaderRecord {
    /// The crashes still counted at `now` (the last day).
    fn recent(&self, now: u64) -> usize {
        self.crashes
            .iter()
            .filter(|&&at| at <= now && now.saturating_sub(at) < DAY)
            .count()
    }
}

impl Guard {
    /// Whether the guard went to `crashed` for [`LOSSES`] device losses
    /// within a day (the newest loss is when it did), not for a start that
    /// died; losses kept from earlier runs alone do not count.
    pub fn crashed_by_losses(&self) -> bool {
        let n = self.losses.len();
        self.state == GuardState::Crashed
            && n >= LOSSES
            && self.losses.last() == Some(&self.at)
            && self.at.saturating_sub(self.losses[n - LOSSES]) < DAY
    }
    /// Whether this record is about `keys`: a current record by the adapter
    /// the mode uses (`keys.key`); one written before 0.5.19 by the first
    /// adapter (`keys.legacy`), as it was then, so an earlier crash keeps
    /// protecting exactly what it protected (L1, record §11).
    pub fn is_for(&self, keys: &Keys) -> bool {
        if self.schema >= KEYED {
            self.key == keys.key
        } else {
            self.key == keys.legacy
        }
    }
    /// The top-level record and the others.
    fn records(&self) -> impl Iterator<Item = &Guard> {
        std::iter::once(self).chain(self.others.iter())
    }
    /// One record, without `others` and the shaders' record.
    fn bare(&self) -> Guard {
        Guard {
            others: Vec::new(),
            shaders: None,
            ..self.clone()
        }
    }
    /// Whether this is no GPU record at all (the shaders' record aside).
    fn is_blank(&self) -> bool {
        self.bare() == Guard::default()
    }
    /// The record about `keys`, if the file has one.
    fn record_for(&self, keys: &Keys) -> Option<Guard> {
        self.records()
            .find(|record| record.is_for(keys) && !record.is_blank())
            .map(Guard::bare)
    }
    /// A record of another key worth keeping: of this player version (a new
    /// one tries again anyway), crashed within [`GUARD_DAYS`], or with a
    /// device loss within the last day.
    fn worth_keeping(&self, app: &str, now: u64) -> bool {
        self.key.app == app
            && ((self.state == GuardState::Crashed
                && now < self.at.saturating_add(GUARD_DAYS * DAY))
                || self.state == GuardState::Attempting
                || self
                    .losses
                    .iter()
                    .any(|&at| at <= now && now.saturating_sub(at) < DAY))
    }
    /// The file with `record` (about `keys`) as the current one, the other
    /// keys' records kept where still worth it, the shaders' record as it
    /// was.
    fn with_current(&self, record: Guard, keys: &Keys, now: u64) -> Guard {
        let others = self
            .records()
            .filter(|old| !old.is_for(keys) && !old.is_blank())
            .filter(|old| old.worth_keeping(&keys.key.app, now))
            .map(Guard::bare)
            .collect();
        Guard {
            others,
            shaders: self.shaders.clone(),
            ..record
        }
    }
    /// Until when (Unix seconds) the mode keyed on `keys` stays off, and
    /// whether for device losses, if it does: the matching record of the
    /// file.
    pub fn blocking(&self, keys: &Keys, now: u64) -> Option<(u64, bool)> {
        self.records().find_map(|record| {
            let until = record.at.saturating_add(GUARD_DAYS * DAY);
            (record.state == GuardState::Crashed && record.is_for(keys) && now < until)
                .then(|| (until, record.crashed_by_losses()))
        })
    }
    /// Until when (Unix seconds) the mode keyed on `keys` stays off (tests).
    #[cfg(test)]
    pub fn blocks(&self, keys: &Keys, now: u64) -> Option<u64> {
        self.blocking(keys, now).map(|(until, _)| until)
    }
}

/// What the guard is keyed on now, and what a file written before 0.5.19
/// was keyed on (the first adapter).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Keys {
    /// The adapter GPU rendering uses (`VESTRA_GPU_ADAPTER` or Windows'
    /// per-app order), its driver, the player.
    pub key: GuardKey,
    /// The same with the system's first adapter.
    pub legacy: GuardKey,
}

#[cfg(test)]
impl Keys {
    /// One key for both (the adapter is the first).
    pub fn same(key: GuardKey) -> Self {
        Self {
            legacy: key.clone(),
            key,
        }
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
    /// Without a path (headless): nothing is read or written.
    pub fn is_off(&self) -> bool {
        self.path.is_none()
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
    /// At start: an `attempting` left by the last run (in any key's record)
    /// becomes `crashed` (now). Returns the guard as it stands.
    pub fn recover(&self, now: u64) -> Guard {
        let _one = one_writer();
        let mut guard = self.read();
        let crash = |record: &mut Guard| {
            if record.state != GuardState::Attempting {
                return false;
            }
            (record.state, record.at) = (GuardState::Crashed, now);
            logged!(
                "render: the last run ended while the GPU started ({} {}); it stays off until the driver or the player changes, or for {GUARD_DAYS} days",
                record.key.adapter,
                record.key.driver
            );
            true
        };
        let mut changed = crash(&mut guard);
        for other in &mut guard.others {
            changed |= crash(other);
        }
        if let Some(record) = guard.shaders.as_mut()
            && record.state == ShaderState::Applying
        {
            record.crashes.retain(|&at| at <= now && now - at < DAY);
            record.crashes.push(now);
            (record.state, record.at, record.skipped) = (ShaderState::Crashed, now, false);
            logged!(
                "shaders: the last run ended while plugin shaders were on ({}); {}",
                record.plugins.join(", "),
                if record.recent(now) >= SHADER_CRASHES {
                    format!(
                        "they stay off until the driver or the player changes, or for {GUARD_DAYS} days"
                    )
                } else {
                    "this start applies none".to_string()
                }
            );
            changed = true;
        }
        if changed {
            self.write(&guard);
        }
        guard
    }
    /// Before GPU rendering (or CPU mode's Direct3D 12 windows) starts, for
    /// the adapter `keys` names. Keep the returned [`Armed`] until the
    /// player ends: dropping it settles the guard, so a start that returns
    /// with an ordinary error (or unwinds) is not taken for a crash; only a
    /// process that dies leaves `attempting` behind. Only this key's record
    /// changes; other keys' records still in force are kept.
    #[must_use = "dropping it settles the guard at once"]
    pub fn arm(&self, keys: &Keys, now: u64) -> Armed {
        let _one = one_writer();
        let file = self.read();
        // Earlier device losses count while the key stays the same.
        let losses = file
            .record_for(keys)
            .map(|record| record.losses)
            .unwrap_or_default();
        let record = Guard {
            schema: SCHEMA,
            state: GuardState::Attempting,
            key: keys.key.clone(),
            at: now,
            losses,
            others: Vec::new(),
            shaders: None,
        };
        self.write(&file.with_current(record, keys, now));
        Armed(self.clone())
    }
    /// Before a non-empty plugin shader list goes to mpv (`plugins`: whose
    /// shaders are on, manifest ids): `applying`. Earlier crashes count
    /// while the key stays the same.
    pub fn shaders_applying(&self, key: &GuardKey, plugins: Vec<String>, now: u64) {
        let _one = one_writer();
        let mut guard = self.read();
        let crashes = guard
            .shaders
            .as_ref()
            .filter(|record| record.key == *key)
            .map(|record| record.crashes.clone())
            .unwrap_or_default();
        guard.shaders = Some(ShaderRecord {
            state: ShaderState::Applying,
            key: key.clone(),
            at: now,
            plugins,
            crashes,
            skipped: false,
        });
        self.write(&guard);
    }
    /// Plugin shaders ran long enough, their lists emptied, or the player
    /// quits: an `applying` becomes `ok`; anything else stays.
    pub fn shaders_settle(&self) {
        let _one = one_writer();
        let mut guard = self.read();
        if let Some(record) = guard.shaders.as_mut()
            && record.state == ShaderState::Applying
        {
            record.state = ShaderState::Ok;
            self.write(&guard);
        }
    }
    /// At start, after [`GuardFile::recover`]: whether this run keeps plugin
    /// shaders off (GPU rendering keyed on `key`). A single crash keeps
    /// them off for this start only (recorded as `skipped`, so the next
    /// start tries again); [`SHADER_CRASHES`] within a day keep them off
    /// until the key changes or [`GUARD_DAYS`] passed.
    pub fn shader_start(&self, key: &GuardKey, now: u64) -> Option<ShaderBlock> {
        let _one = one_writer();
        let mut guard = self.read();
        let record = guard.shaders.as_mut()?;
        if record.state != ShaderState::Crashed || record.key != *key {
            return None;
        }
        let until = if record.recent(record.at) >= SHADER_CRASHES {
            let until = record.at.saturating_add(GUARD_DAYS * DAY);
            if now >= until {
                return None;
            }
            Some(until)
        } else if record.skipped {
            return None;
        } else {
            record.skipped = true;
            None
        };
        let plugins = record.plugins.clone();
        if until.is_none() {
            self.write(&guard);
        }
        Some(ShaderBlock { plugins, until })
    }
    /// The GPU device was lost (the player restarts): record it for `keys`;
    /// the [`LOSSES`]th loss within a day for the same key turns its record
    /// to `crashed`. Returns whether it did. Only this key's record changes
    /// (rewritten with the current key and schema).
    pub fn device_lost(&self, keys: &Keys, now: u64) -> bool {
        let _one = one_writer();
        let file = self.read();
        let mut guard = file.record_for(keys).unwrap_or_default();
        (guard.schema, guard.key) = (SCHEMA, keys.key.clone());
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
        self.write(&file.with_current(guard, keys, now));
        crashed
    }
    /// GPU rendering started (or gave up on its own, or the player quits):
    /// an `attempting` becomes `ok`; anything else stays.
    pub fn settle(&self) {
        let _one = one_writer();
        let mut guard = self.read();
        if guard.state == GuardState::Attempting {
            guard.state = GuardState::Ok;
            self.write(&guard);
        }
    }
}

/// One read-modify-write of `render-state.json` at a time in this process:
/// the device's callback (`device_lost`, any thread) and the UI thread's
/// settle timer must not lose each other's writes. One player writes the
/// file at a time (a restarted one waits for the old one to end).
fn one_writer() -> std::sync::MutexGuard<'static, ()> {
    static WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());
    WRITER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Main-window draws after which CPU mode's Direct3D 12 windows count as
/// started when no video is being opened.
pub const FIRST_DRAWS: u64 = 3;

/// Whether CPU mode's Direct3D 12 windows have started, so the crash guard
/// settles (record 2026-10-render-modes §11.4, §11.5): the first video
/// picture was shown (`pictures`, the presentation's presented frames: its
/// upload into the windows' texture went through), or the window drew
/// [`FIRST_DRAWS`] times and no source is being opened (`opening`: a start
/// with a file, or a source loading), whose first upload would still come.
/// A start that never gets there (audio only, an open that fails) settles
/// on the 5 s timer, as GPU mode does.
pub fn windows_started(draws: u64, pictures: u64, opening: bool) -> bool {
    pictures >= 1 || (draws >= FIRST_DRAWS && !opening)
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
        std::mem::forget(file.arm(&Keys::same(key("31.0.14005.21001")), 100));
        // The run died: the next start finds `attempting`.
        let guard = file.recover(200);
        assert_eq!((guard.state, guard.at), (GuardState::Crashed, 200));
        assert_eq!(file.read(), guard, "recorded");
        let until = 200 + GUARD_DAYS * DAY;
        assert_eq!(
            guard.blocks(&Keys::same(key("31.0.14005.21001")), 300),
            Some(until)
        );
        assert_eq!(
            guard.blocks(&Keys::same(key("32.0.32015.7029")), 300),
            None,
            "driver updated"
        );
        let mut player = key("31.0.14005.21001");
        player.app = "0.5.14".into();
        assert_eq!(
            guard.blocks(&Keys::same(player.clone()), 300),
            None,
            "player updated"
        );
        assert_eq!(
            guard.blocks(&Keys::same(key("31.0.14005.21001")), until),
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
        let armed = file.arm(&Keys::same(key("1")), 100);
        file.settle();
        assert_eq!(file.read().state, GuardState::Ok);
        drop(armed);
        assert_eq!(file.recover(200).state, GuardState::Ok);
        // A start that returns early with an error drops the guard: settled.
        let start = || -> Result<(), String> {
            let _armed = file.arm(&Keys::same(key("1")), 250);
            assert_eq!(file.read().state, GuardState::Attempting);
            Err("Backend::start failed".into())
        };
        assert!(start().is_err());
        assert_eq!(file.read().state, GuardState::Ok, "an error is not a crash");
        std::mem::forget(file.arm(&Keys::same(key("1")), 300));
        file.recover(400);
        file.settle();
        assert_eq!(file.read().state, GuardState::Crashed);
        let none = GuardFile::new(None);
        drop(none.arm(&Keys::same(key("1")), 1));
        assert_eq!(none.recover(2), Guard::default());
    }

    /// Branch §1.1's restart after a lost device, now through the guard:
    /// the second loss within a day turns GPU rendering off.
    #[test]
    fn a_second_device_loss_within_a_day_blocks_like_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let armed = file.arm(&Keys::same(key("1")), 100);
        assert!(
            !file.device_lost(&Keys::same(key("1")), 110),
            "one loss: restart on GPU"
        );
        assert_eq!(file.read().state, GuardState::Ok, "not taken for a crash");
        std::mem::forget(armed);
        // The restarted player arms again: the loss is kept.
        std::mem::forget(file.arm(&Keys::same(key("1")), 120));
        assert_eq!(file.read().losses, [110]);
        assert!(
            file.device_lost(&Keys::same(key("1")), 130),
            "the second one"
        );
        let guard = file.recover(140);
        assert_eq!(guard.state, GuardState::Crashed);
        assert_eq!(
            guard.blocks(&Keys::same(key("1")), 140),
            Some(130 + GUARD_DAYS * DAY)
        );
        assert_eq!(
            guard.blocks(&Keys::same(key("2")), 140),
            None,
            "driver updated"
        );
        assert!(guard.crashed_by_losses());
        // A start that dies later is not reported as losses (stale ones
        // kept by `arm`).
        std::mem::forget(file.arm(&Keys::same(key("1")), 200));
        let later = file.recover(300);
        assert_eq!(later.state, GuardState::Crashed);
        assert!(!later.crashed_by_losses(), "{later:?}");
        // A day apart, or another driver: one loss each.
        let other = GuardFile::new(Some(dir.path().join("other.json")));
        assert!(!other.device_lost(&Keys::same(key("1")), 100));
        assert!(!other.device_lost(&Keys::same(key("1")), 100 + DAY));
        assert!(!other.device_lost(&Keys::same(key("2")), 100 + DAY + 1));
        assert_eq!(other.read().losses, [100 + DAY + 1]);
    }

    /// L1 (record §11): a file written before 0.5.19 (no `schema`) was keyed
    /// on the first adapter; it keeps blocking exactly that, and the next
    /// write keys it on the adapter GPU rendering uses.
    #[test]
    fn a_guard_file_from_before_the_adapter_key_is_read_as_it_was_meant() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("render-state.json");
        let first = key("31.0.14005.21001");
        let nvidia = GuardKey {
            adapter: "NVIDIA GeForce RTX 4070 Laptop GPU".into(),
            driver: "32.0.16.1062".into(),
            app: "0.5.13".into(),
        };
        // As 0.5.18 wrote it: crashed, keyed on the first adapter (the
        // 780M), with VESTRA_GPU_ADAPTER on the 4070 at the time.
        std::fs::write(
            &path,
            br#"{"state":"crashed","adapter":"AMD Radeon 780M","driver":"31.0.14005.21001","app":"0.5.13","at":100,"losses":[90,100]}"#,
        )
        .unwrap();
        let file = GuardFile::new(Some(path.clone()));
        let keys = Keys {
            key: nvidia.clone(),
            legacy: first.clone(),
        };
        let old = file.recover(200);
        assert_eq!(old.schema, 0);
        assert!(old.blocks(&keys, 200).is_some(), "still blocks");
        assert!(
            old.blocks(&Keys::same(nvidia.clone()), 200).is_none(),
            "not by the new key"
        );
        // The first device loss after the update: the losses carry over
        // (the same machine and setup), and the record is now keyed on the
        // 4070.
        assert!(file.device_lost(&keys, 150));
        let new = file.read();
        assert_eq!((new.schema, &new.key), (SCHEMA, &nvidia));
        assert_eq!(new.losses, [90, 100, 150]);
        assert!(new.blocks(&keys, 200).is_some());
        assert!(new.blocks(&Keys::same(first.clone()), 200).is_none());
        // Arming keys the record on the GPU's adapter too.
        std::mem::forget(file.arm(&keys, 300));
        let armed = file.read();
        assert_eq!((armed.schema, armed.key), (SCHEMA, nvidia));
        // A broken or empty file is no record.
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(file.recover(400), Guard::default());
    }

    /// Re-review M (record §11.5): a start with a file waits for the first
    /// picture, whose upload into the Direct3D 12 texture comes hundreds of
    /// ms after the first draws.
    #[test]
    fn direct3d_12_windows_count_as_started_after_their_first_picture_when_opening() {
        // Nothing opened: three draws.
        assert!(!windows_started(2, 0, false));
        assert!(windows_started(FIRST_DRAWS, 0, false));
        // A file opening: draws alone are not enough.
        assert!(!windows_started(60, 0, true));
        assert!(windows_started(1, 1, true), "the first picture was shown");
        assert!(windows_started(0, 1, false));
    }

    /// Review item 1 (record §12): on a hybrid laptop GPU rendering (the
    /// 4070) and CPU mode's Direct3D 12 windows (the 780M) have their own
    /// keys; arming, settling or losing one must not erase the other's
    /// crash, or every other start would crash again.
    #[test]
    fn each_adapter_keeps_its_own_record() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let gpu = Keys::same(GuardKey {
            adapter: "NVIDIA GeForce RTX 4070 Laptop GPU".into(),
            driver: "32.0.16.1062".into(),
            app: "0.5.23".into(),
        });
        let windows = Keys::same(GuardKey {
            adapter: "AMD Radeon 780M".into(),
            driver: "32.0.32015.7029".into(),
            app: "0.5.23".into(),
        });
        // GPU rendering crashes on the 4070.
        std::mem::forget(file.arm(&gpu, 100));
        assert!(file.recover(200).blocks(&gpu, 200).is_some());
        // The next start renders in software with the 780M's windows,
        // which start fine.
        drop(file.arm(&windows, 210));
        let guard = file.recover(300);
        assert!(guard.blocks(&gpu, 300).is_some(), "the 4070 crash is kept");
        assert!(guard.blocks(&windows, 300).is_none());
        // A device loss of the windows does not erase it either.
        assert!(!file.device_lost(&windows, 310));
        assert!(file.recover(320).blocks(&gpu, 320).is_some());
        // The other way round: the windows crash on the 780M, then GPU
        // rendering on the 4070 starts and settles.
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        std::mem::forget(file.arm(&windows, 100));
        assert!(file.recover(200).blocks(&windows, 200).is_some());
        drop(file.arm(&gpu, 210));
        assert!(!file.device_lost(&gpu, 220), "one loss");
        let guard = file.recover(300);
        assert!(
            guard.blocks(&windows, 300).is_some(),
            "the 780M crash is kept"
        );
        assert!(guard.blocks(&gpu, 300).is_none());
        assert_eq!(guard.others.len(), 1);
        // The 4070's loss is kept too while the windows arm again.
        drop(file.arm(&windows, 400));
        let guard = file.read();
        let gpu_record = guard.others.iter().find(|r| r.is_for(&gpu)).unwrap();
        assert_eq!(gpu_record.losses, [220]);
        // Records that no longer matter go: expired, or of another player.
        let later = 200 + GUARD_DAYS * DAY;
        drop(file.arm(&gpu, later));
        assert!(file.read().others.is_empty(), "{:?}", file.read());
        let mut older = gpu.clone();
        older.key.app = "0.5.22".into();
        std::mem::forget(file.arm(&older, later + 10));
        file.recover(later + 20);
        drop(file.arm(&gpu, later + 30));
        assert!(
            file.read().others.is_empty(),
            "another player version's crash"
        );
    }

    /// A player before 0.5.23 (schema 2, no `others`) reads a schema-3 file:
    /// the top-level record is what it sees, the others are ignored.
    #[test]
    fn an_older_player_reads_the_current_record_of_a_schema_3_file() {
        #[derive(Debug, Deserialize)]
        struct Schema2 {
            schema: u32,
            state: GuardState,
            #[serde(flatten)]
            key: GuardKey,
            at: u64,
            #[serde(default)]
            losses: Vec<u64>,
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("render-state.json");
        let file = GuardFile::new(Some(path.clone()));
        let other = Keys::same(key("other"));
        std::mem::forget(file.arm(&other, 100));
        file.recover(150);
        assert!(!file.device_lost(&Keys::same(key("1")), 200));
        let text = std::fs::read(&path).unwrap();
        assert!(String::from_utf8_lossy(&text).contains("\"others\""));
        let old: Schema2 = serde_json::from_slice(&text).unwrap();
        assert_eq!(
            (old.schema, old.state, old.key, old.at, old.losses),
            (SCHEMA, GuardState::Ok, key("1"), 0, vec![200])
        );
    }

    /// Plugin shaders: a run that ends while they are on keeps them off for
    /// the next start only; a second such run within a day keeps them off
    /// until the driver or the player changes, or for `GUARD_DAYS`.
    #[test]
    fn a_run_that_died_with_plugin_shaders_on_keeps_them_off_at_the_next_start() {
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        let gpu = key("1");
        assert_eq!(file.shader_start(&gpu, 100), None, "no file");
        // Applied, ran, settled: nothing at the next start.
        file.shaders_applying(&gpu, vec!["acme.sharpen".into()], 100);
        assert_eq!(file.read().shaders.unwrap().state, ShaderState::Applying);
        file.shaders_settle();
        assert_eq!(file.recover(200).shaders.unwrap().state, ShaderState::Ok);
        assert_eq!(file.shader_start(&gpu, 200), None);
        // Applied, then the process died.
        file.shaders_applying(&gpu, vec!["acme.sharpen".into()], 300);
        let recovered = file.recover(400).shaders.unwrap();
        assert_eq!(
            (recovered.state, recovered.at, &recovered.crashes[..]),
            (ShaderState::Crashed, 400, &[400][..])
        );
        // Another driver or player: shaders apply.
        assert_eq!(file.shader_start(&key("2"), 400), None);
        let block = file.shader_start(&gpu, 400).unwrap();
        assert_eq!(
            block,
            ShaderBlock {
                plugins: vec!["acme.sharpen".into()],
                until: None
            }
        );
        // That start kept them off; the one after tries again.
        assert!(file.read().shaders.unwrap().skipped);
        assert_eq!(file.shader_start(&gpu, 500), None);
        // It dies again within a day: off for days.
        file.shaders_applying(&gpu, vec!["acme.sharpen".into()], 600);
        file.recover(700);
        let until = 700 + GUARD_DAYS * DAY;
        assert_eq!(file.shader_start(&gpu, 800).unwrap().until, Some(until));
        assert_eq!(file.shader_start(&gpu, 900).unwrap().until, Some(until));
        assert_eq!(file.shader_start(&gpu, until), None, "expired");
        assert_eq!(file.shader_start(&key("2"), 800), None, "driver updated");
        // A crash a day after the last one counts alone again.
        let dir = tempfile::tempdir().unwrap();
        let file = GuardFile::new(Some(dir.path().join("render-state.json")));
        file.shaders_applying(&gpu, vec![], 100);
        file.recover(200);
        file.shaders_applying(&gpu, vec![], 200 + DAY);
        file.recover(300 + DAY);
        assert_eq!(file.read().shaders.unwrap().crashes, [300 + DAY]);
        assert_eq!(file.shader_start(&gpu, 400 + DAY).unwrap().until, None);
        // Headless: nothing.
        let none = GuardFile::new(None);
        none.shaders_applying(&gpu, vec![], 1);
        assert_eq!(none.recover(2), Guard::default());
        assert_eq!(none.shader_start(&gpu, 3), None);
    }

    /// The GPU records and the shaders' record share the file: writing one
    /// keeps the other.
    #[test]
    fn gpu_and_shader_records_keep_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("render-state.json");
        let file = GuardFile::new(Some(path.clone()));
        let gpu = Keys::same(key("1"));
        let armed = file.arm(&gpu, 100);
        file.shaders_applying(&gpu.key, vec!["a".into()], 110);
        file.settle();
        drop(armed);
        let guard = file.read();
        assert_eq!(guard.state, GuardState::Ok);
        assert_eq!(guard.shaders.as_ref().unwrap().state, ShaderState::Applying);
        // A device loss and a new arm keep the shaders' record.
        assert!(!file.device_lost(&gpu, 120));
        std::mem::forget(file.arm(&Keys::same(key("2")), 130));
        let guard = file.read();
        assert_eq!(guard.shaders.as_ref().unwrap().plugins, ["a"]);
        assert!(guard.others.iter().all(|other| other.shaders.is_none()));
        // Both died: both are crashes.
        let guard = file.recover(140);
        assert_eq!(guard.state, GuardState::Crashed);
        assert_eq!(guard.shaders.unwrap().state, ShaderState::Crashed);
        // A file with only the shaders' record holds no GPU record.
        std::fs::write(&path, br#"{"shaders":{"state":"applying","adapter":"x","driver":"1","app":"0.5.13","at":5,"plugins":["a"]}}"#).unwrap();
        let guard = file.read();
        assert!(guard.is_blank());
        assert!(guard.record_for(&gpu).is_none());
        assert!(guard.blocks(&gpu, 6).is_none());
        // A player before 0.6.3 (no `shaders` field) reads such a file.
        #[derive(Deserialize)]
        struct Before {
            state: GuardState,
            #[serde(flatten)]
            key: GuardKey,
        }
        let old: Before = serde_json::from_slice(
            br#"{"schema":3,"state":"crashed","adapter":"x","driver":"1","app":"0.6.2","at":0,"shaders":{"state":"crashed"}}"#,
        )
        .unwrap();
        assert_eq!(
            (old.state, old.key.app.as_str()),
            (GuardState::Crashed, "0.6.2")
        );
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
