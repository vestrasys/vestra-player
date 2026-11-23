//! Guest windows (doc/architecture/guest-windows.md, user ruling
//! 2026-11-21): "Play in New Window" starts another `vestra-host` with
//! `--guest`, a player with a temporary profile of its own under
//! `<app cache>/guests/<uuid>`. Its profile directory replaces the
//! application data directory (`platform::app_data_dir`); it starts from a
//! snapshot of the main profile's settings (`snapshot`), runs the main
//! profile's packages read-only (`packages::package_store::Store::open_read_only`),
//! takes no part in the single-instance lock, lists the package versions it
//! runs from so that the main player keeps them (`lease`, `leased`), and
//! deletes its directory when it ends (`Session`); a directory a crash left
//! behind is swept at the next start of any player (`sweep`).
#![forbid(unsafe_code)]
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Held (an operating-system lock) for the guest's life: a directory whose
/// lock another process can take is a leftover.
const LOCK: &str = "guest.lock";
/// The package digests the guest runs from (a JSON list of strings).
const LEASE: &str = "packages.json";
/// The guest's application data directory.
const PROFILE: &str = "profile";
/// Under the cache directory.
const GUESTS: &str = "guests";
/// The files `app.openWindow` takes at most (one command line).
pub const MAX_PATHS: usize = 64;
/// How far a guest's window is moved from the main window's frame (logical
/// pixels, and desktop pixels for the Windows placement), as Windows and
/// browsers cascade a new window.
const CASCADE: i64 = 32;
/// A directory without a lock file is a guest that died before it took
/// one, once nothing changed in it for this long.
const UNLOCKED_KEPT: Duration = Duration::from_secs(60);
/// How long a new guest waits for its lock while a sweep probes it.
const CREATE_WAIT: Duration = Duration::from_secs(5);
/// How long a restarted guest waits for the one it replaces to let go.
const ADOPT_WAIT: Duration = Duration::from_secs(20);
/// The settings snapshot (doc/architecture/guest-windows.md §3): copied
/// from the main profile when present. `ui-state.json` is copied apart
/// (`ui_state_snapshot`).
const SNAPSHOT: [&str; 3] = ["render.json", "render-state.json", "render-denylist.json"];
const UI_STATE: &str = "ui-state.json";
/// A `ui-state.json` larger than this is not copied (the player's own is
/// a few kilobytes).
const MAX_UI_STATE: u64 = 4 * 1024 * 1024;

struct Guest {
    directory: PathBuf,
    /// Held until the process ends.
    _lock: File,
}

static GUEST: OnceLock<Guest> = OnceLock::new();
/// `--silent`: the guests this player starts are silent too (tests).
static SILENT: AtomicBool = AtomicBool::new(false);

/// This process is a guest.
pub fn active() -> bool {
    GUEST.get().is_some()
}

/// This guest's directory (`<app cache>/guests/<uuid>`).
pub fn directory() -> Option<&'static Path> {
    GUEST.get().map(|guest| guest.directory.as_path())
}

/// This guest's application data directory (`platform::app_data_dir`).
pub fn profile() -> Option<PathBuf> {
    directory().map(|directory| directory.join(PROFILE))
}

/// Where guests live: `<app cache>/guests`.
pub fn root() -> Option<PathBuf> {
    crate::platform::app_cache_dir().map(|directory| directory.join(GUESTS))
}

/// The title with the guest marker (`{} (Temporary Window)`, translated),
/// or as it is in a main player.
pub fn decorate(title: &str) -> String {
    if active() {
        crate::i18n::tr_args("{} (Temporary Window)", &serde_json::json!([title]))
    } else {
        title.to_owned()
    }
}

/// Remember this player's `--silent` for the guests it starts.
pub fn set_silent(silent: bool) {
    SILENT.store(silent, Ordering::Relaxed);
}

/// Deletes the guest's directory when the player ends (`player()` returns),
/// unless the player restarts itself: the new process adopts it
/// (`--vestra-guest-dir`).
pub struct Session(());

impl Drop for Session {
    fn drop(&mut self) {
        let Some(directory) = directory() else {
            return;
        };
        if crate::host::restarting() {
            diag!(
                "guest: kept {} for the restarted player",
                directory.display()
            );
            return;
        }
        // The lock file is open (the standard library shares it for
        // deletion), so the directory goes with it.
        if let Err(error) = fs::remove_dir_all(directory) {
            diag!(
                "guest: {} left for the next start ({error})",
                directory.display()
            );
        }
    }
}

/// Become a guest: a new directory (or `adopt`'s, a guest restarting
/// itself), its lock, the settings snapshot, the profile override. Then
/// sweep the leftovers of guests that ended without cleaning up.
pub fn start(adopt: Option<PathBuf>) -> Result<Session> {
    let root = root().context("no cache directory for a temporary window")?;
    fs::create_dir_all(&root).with_context(|| format!("{}", root.display()))?;
    let adopted = adopt.and_then(|directory| match adopt_directory(&root, &directory) {
        Ok(guest) => Some(guest),
        Err(error) => {
            diag!(
                "guest: {} cannot be adopted ({error:#}); starting a new one",
                directory.display()
            );
            None
        }
    });
    let guest = match adopted {
        Some(guest) => guest,
        None => {
            let guest = create(&root)?;
            // Read before the override: the main profile's files.
            snapshot(
                crate::platform::main_app_data_dir().as_deref(),
                &crate::ui::ui_state::default_path(),
                &guest.directory.join(PROFILE),
            );
            guest
        }
    };
    let directory = guest.directory.clone();
    if GUEST.set(guest).is_err() {
        anyhow::bail!("this player is already a temporary window");
    }
    diag!("guest: profile {}", directory.display());
    let _ = std::thread::Builder::new()
        .name("guest-sweep".into())
        .spawn(move || sweep(&root, Some(&directory)));
    Ok(Session(()))
}

/// A main player's start: sweep the leftovers of guests, on a thread.
pub fn sweep_later() {
    let Some(root) = root() else {
        return;
    };
    let _ = std::thread::Builder::new()
        .name("guest-sweep".into())
        .spawn(move || sweep(&root, None));
}

/// A new guest directory under `root`, locked. A sweep that took a new
/// directory's lock before this process did may have deleted it (it was
/// not locked yet, so it looked like a leftover): then another one.
fn create(root: &Path) -> Result<Guest> {
    for _ in 0..8 {
        let directory = root.join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir(&directory).with_context(|| format!("{}", directory.display()))?;
        if let Some(guest) = create_at(&directory)? {
            return Ok(guest);
        }
        diag!(
            "guest: {} was swept while it started; trying another",
            directory.display()
        );
    }
    anyhow::bail!(
        "no temporary window directory could be made under {}",
        root.display()
    )
}

/// Lock the new directory `directory` and make its profile; `None` when a
/// sweep deleted it meanwhile. A probe holds a lock for a moment
/// (`probe`): `CREATE_WAIT` for it.
fn create_at(directory: &Path) -> Result<Option<Guest>> {
    let lock = open_lock(directory)?;
    lock_within(&lock, CREATE_WAIT)
        .map_err(|error| anyhow::anyhow!("{}: {error}", directory.display()))?;
    // Locked: no sweep deletes it from now on. One that had the lock first
    // may have deleted it already.
    if !still_there(directory) {
        return Ok(None);
    }
    fs::create_dir(directory.join(PROFILE))?;
    Ok(Some(Guest {
        directory: directory.to_path_buf(),
        _lock: lock,
    }))
}

/// `directory` (a guest that restarted itself): under `root`, its profile
/// still there, its lock taken once the old process let go.
fn adopt_directory(root: &Path, directory: &Path) -> Result<Guest> {
    let canonical = dunce::canonicalize(directory)?;
    anyhow::ensure!(
        canonical.parent() == Some(dunce::canonicalize(root)?.as_path()),
        "not a temporary window's directory"
    );
    let lock = open_lock(&canonical)?;
    lock_within(&lock, ADOPT_WAIT)?;
    // A sweep may have taken it between the old process and this one.
    anyhow::ensure!(
        still_there(&canonical) && canonical.join(PROFILE).is_dir(),
        "its profile is gone"
    );
    Ok(Guest {
        directory: canonical,
        _lock: lock,
    })
}

/// Take `lock`, waiting up to `wait` while another process holds it.
fn lock_within(lock: &File, wait: Duration) -> io::Result<()> {
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another process holds the lock",
                ));
            }
            Err(TryLockError::Error(error)) => return Err(error),
        }
    }
}

/// The directory and its lock file are still where they were (a sweep
/// deletes both; it never makes a lock file).
fn still_there(directory: &Path) -> bool {
    directory.is_dir() && directory.join(LOCK).is_file()
}

fn open_lock(directory: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(LOCK))
}

/// Copy the settings snapshot from the main profile `main` (and its window
/// state file `ui_state`) into `profile` (doc/architecture/guest-windows.md
/// §3). A file that is missing or cannot be copied is left out: the guest
/// starts with that file's defaults.
pub fn snapshot(main: Option<&Path>, ui_state: &Path, profile: &Path) {
    if let Some(main) = main {
        for name in SNAPSHOT {
            let from = main.join(name);
            if from.is_file()
                && let Err(error) = fs::copy(&from, profile.join(name))
            {
                diag!("guest: {name} not copied ({error})");
            }
        }
    }
    match ui_state_snapshot(ui_state) {
        Ok(Some(bytes)) => {
            if let Err(error) = fs::write(profile.join(UI_STATE), bytes) {
                diag!("guest: {UI_STATE} not copied ({error})");
            }
        }
        Ok(None) => {}
        Err(error) => diag!("guest: {UI_STATE} not copied ({error:#})"),
    }
}

/// The main window state file as the guest starts from it: without the
/// palette's recent commands (usage history), the window cascaded. `None`
/// without a file.
fn ui_state_snapshot(path: &Path) -> Result<Option<Vec<u8>>> {
    let bytes = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_UI_STATE => fs::read(path)?,
        Ok(_) => anyhow::bail!("not a file of at most {MAX_UI_STATE} bytes"),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let mut value: Value = serde_json::from_slice(&bytes)?;
    let state = value.as_object_mut().context("not an object")?;
    state.remove("paletteRecent");
    for key in ["window", "windowPhysical"] {
        if let Some(frame) = state.get_mut(key).and_then(Value::as_object_mut) {
            for axis in ["x", "y"] {
                if let Some(slot) = frame.get_mut(axis) {
                    if let Some(at) = slot.as_i64() {
                        *slot = Value::from(at.saturating_add(CASCADE));
                    } else if let Some(at) = slot.as_f64() {
                        *slot = Value::from(at + CASCADE as f64);
                    }
                }
            }
        }
    }
    Ok(Some(serde_json::to_vec(&value)?))
}

/// The guest's lease: the package digests it runs from, written beside
/// its lock before any plugin of theirs starts (`package_manager.rs`). The
/// main player's library cleanup leaves them (`leased`). Nothing in a main
/// player.
pub fn lease(digests: &BTreeSet<String>) -> io::Result<()> {
    let Some(directory) = directory() else {
        return Ok(());
    };
    write_lease(directory, digests)
}

fn write_lease(directory: &Path, digests: &BTreeSet<String>) -> io::Result<()> {
    let partial = directory.join(format!("{LEASE}.partial"));
    fs::write(
        &partial,
        serde_json::to_vec(digests).map_err(io::Error::other)?,
    )?;
    fs::rename(&partial, directory.join(LEASE))
}

/// What the main player's library cleanup must keep for the running
/// guests (`leased_in`).
#[derive(Debug, PartialEq)]
pub enum Leases {
    /// These digests.
    These(BTreeSet<String>),
    /// Everything, this time: a guest that may be running has a lease
    /// that cannot be read (not written yet, or broken).
    All,
}

impl Leases {
    /// Whether `digest` stays.
    pub fn keeps(&self, digest: &str) -> bool {
        match self {
            Self::These(digests) => digests.contains(digest),
            Self::All => true,
        }
    }
}

/// The package digests the guests under `root` that may be running use:
/// the leases of the directories whose lock this process cannot take.
/// Fail-safe: such a directory whose lease cannot be read or parsed, or a
/// guests directory that cannot be listed, keeps everything for this pass.
pub fn leased_in(root: &Path) -> Leases {
    let mut digests = BTreeSet::new();
    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Leases::These(digests),
        Err(_) => return Leases::All,
    };
    for entry in entries {
        let Ok(entry) = entry else {
            return Leases::All;
        };
        let directory = entry.path();
        if !directory.is_dir() || matches!(probe(&directory), Probe::Dead(_)) {
            continue;
        }
        let listed = fs::read(directory.join(LEASE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<BTreeSet<String>>(&bytes).ok());
        match listed {
            Some(listed) => digests.extend(listed),
            None => return Leases::All,
        }
    }
    Leases::These(digests)
}

/// `leased_in` under the guests directory `root` (`None`: none, as in the
/// host's unit tests, which never read the user's).
pub fn leased(root: Option<&Path>) -> Leases {
    root.map_or(Leases::These(BTreeSet::new()), leased_in)
}

/// What a look at a guest directory's lock found.
enum Probe {
    /// A process holds it.
    Live,
    /// Nobody did: this process holds it now, until the value drops.
    Dead(#[allow(dead_code)] File),
    /// No lock file, or it cannot be opened or locked.
    Unknown,
}

/// Try `directory`'s lock (never making the lock file).
fn probe(directory: &Path) -> Probe {
    let Ok(lock) = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join(LOCK))
    else {
        return Probe::Unknown;
    };
    match lock.try_lock() {
        Ok(()) => Probe::Dead(lock),
        Err(TryLockError::WouldBlock) => Probe::Live,
        Err(TryLockError::Error(_)) => Probe::Unknown,
    }
}

/// Delete the directories under `root` no guest holds (but `own`): those
/// whose lock this process takes, deleted while it holds that lock (a
/// guest starting there cannot lock it meanwhile, and finds it gone
/// afterwards, `create_at`), and those without a lock file that nothing
/// changed for `UNLOCKED_KEPT`. How many went.
pub fn sweep(root: &Path, own: Option<&Path>) -> usize {
    let Ok(entries) = fs::read_dir(root) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let directory = entry.path();
        if own == Some(directory.as_path()) || !directory.is_dir() {
            continue;
        }
        let held = match probe(&directory) {
            Probe::Live => continue,
            Probe::Dead(lock) => Some(lock),
            Probe::Unknown => {
                let old = !directory.join(LOCK).exists()
                    && fs::metadata(&directory)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age >= UNLOCKED_KEPT);
                if !old {
                    continue;
                }
                None
            }
        };
        // The lock file is open here (the standard library shares it for
        // deletion), so it goes with the directory.
        match fs::remove_dir_all(&directory) {
            Ok(()) => removed += 1,
            Err(error) => diag!(
                "guest: {} left for a later start ({error})",
                directory.display()
            ),
        }
        drop(held);
    }
    if removed > 0 {
        diag!("guest: removed {removed} temporary window folder(s) left behind");
    }
    removed
}

/// What a guest does not take from the environment of the player that
/// starts it: the test and diagnostic switches that write files of that
/// run (traces, recordings, shots, logs) or script its input.
/// `VESTRA_TEST_OFFSCREEN` and `VESTRA_TEST_DISPLAYS` stay: a guest of an
/// offscreen run is offscreen too.
const NOT_INHERITED: [&str; 24] = [
    "VESTRA_LOG_FILE",
    "VESTRA_TRACE",
    "VESTRA_TEST_WIRE_TRACE",
    "VESTRA_UI_PROPERTY_TRACE",
    "VESTRA_UI_RECORD",
    "VESTRA_UI_SHOTS",
    "VESTRA_WIDGET_SHOTS",
    "VESTRA_SHEET_SHOTS",
    "VESTRA_STRIP_SHOTS",
    "VESTRA_PROBE_OUT",
    "VESTRA_PIP_LOG",
    "VESTRA_FRAME_LOG",
    "VESTRA_UPDATE_MPV_RULES",
    "VESTRA_TEST_LOG_DIR",
    "VESTRA_TEST_EXECUTE",
    "VESTRA_TEST_KEY",
    "VESTRA_TEST_PARK",
    "VESTRA_TEST_WAKE",
    "VESTRA_TEST_UI_STALL",
    "VESTRA_TEST_CORE_PANIC",
    "VESTRA_TEST_GPU_STALL",
    "VESTRA_TEST_REMOVE_DEVICE",
    "VESTRA_TEST_CONFIRM",
    "VESTRA_INSTANCE_DIR",
];

/// Start a guest on `paths` (`app.openWindow`): `vestra-host --guest
/// [--silent] -- <paths>`; its process id. It inherits this player's
/// environment but `NOT_INHERITED` (the switches that name the main
/// profile's files are ignored by a guest, §1 of the design).
pub fn spawn(paths: &[PathBuf]) -> io::Result<u32> {
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(exe);
    for key in NOT_INHERITED {
        command.env_remove(key);
    }
    crate::platform::unscoped_child_env(&mut command);
    command.arg("--guest");
    if SILENT.load(Ordering::Relaxed) {
        command.arg("--silent");
    }
    command
        .arg("--")
        .args(paths)
        .stdin(std::process::Stdio::null());
    // Windows: the new window may take the foreground from this one.
    crate::platform::allow_handoff_foreground();
    let mut child = command.spawn()?;
    let pid = child.id();
    // Reaped when it ends (a zombie elsewhere than on Windows otherwise);
    // never waited for by the player.
    let _ = std::thread::Builder::new()
        .name("guest-wait".into())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path) -> Vec<(String, Vec<u8>)> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let name = path.strip_prefix(root).unwrap().to_string_lossy().into();
                    files.push((name, fs::read(&path).unwrap()));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn the_snapshot_copies_the_settings_without_history_and_cascades_the_window() {
        let main = tempfile::tempdir().unwrap();
        let profile = tempfile::tempdir().unwrap();
        let ui = main.path().join(UI_STATE);
        fs::write(
            &ui,
            serde_json::json!({"window":{"x":100.5,"y":50.0,"width":800.0,"height":600.0},
                "windowPhysical":{"x":-10,"y":20,"width":1200,"height":900},
                "appearance":{"theme":"light","language":"de"},
                "bindings":{"host/play-pause":["K"]},
                "paletteRecent":["host/stop"]})
            .to_string(),
        )
        .unwrap();
        fs::write(
            main.path().join("render.json"),
            b"{\"preference\":\"software\"}",
        )
        .unwrap();
        fs::write(main.path().join("render-state.json"), b"guard").unwrap();
        fs::write(main.path().join("media-sources.json"), b"{}").unwrap();
        fs::create_dir(main.path().join("plugin-data")).unwrap();
        fs::write(main.path().join("plugin-data").join("x"), b"x").unwrap();
        let before = tree(main.path());
        snapshot(Some(main.path()), &ui, profile.path());
        assert_eq!(tree(main.path()), before, "the main profile is only read");
        let copied: Vec<_> = tree(profile.path()).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            copied,
            ["render-state.json", "render.json", "ui-state.json"]
        );
        let state: Value =
            serde_json::from_slice(&fs::read(profile.path().join(UI_STATE)).unwrap()).unwrap();
        assert_eq!(state["window"]["x"], 132.5);
        assert_eq!(state["window"]["y"], 82.0);
        assert_eq!(state["window"]["width"], 800.0);
        assert_eq!(state["windowPhysical"]["x"], 22);
        assert_eq!(state["windowPhysical"]["y"], 52);
        assert_eq!(state["appearance"]["language"], "de");
        assert_eq!(state["bindings"]["host/play-pause"][0], "K");
        assert!(state.get("paletteRecent").is_none());
        // Nothing to copy: nothing copied; a broken file: left out.
        let empty = tempfile::tempdir().unwrap();
        snapshot(None, &empty.path().join(UI_STATE), empty.path());
        assert!(tree(empty.path()).is_empty());
        fs::write(&ui, b"not json").unwrap();
        let broken = tempfile::tempdir().unwrap();
        snapshot(None, &ui, broken.path());
        assert!(tree(broken.path()).is_empty());
    }

    #[test]
    fn leases_count_while_their_guest_holds_its_lock_and_dead_guests_are_swept() {
        let root = tempfile::tempdir().unwrap();
        assert_eq!(
            leased_in(&root.path().join("none")),
            Leases::These(BTreeSet::new()),
            "no guests directory"
        );
        let live = create(root.path()).unwrap();
        write_lease(&live.directory, &BTreeSet::from(["a".repeat(64)])).unwrap();
        let dead = create(root.path()).unwrap();
        write_lease(&dead.directory, &BTreeSet::from(["b".repeat(64)])).unwrap();
        let dead_directory = dead.directory.clone();
        drop(dead);
        let leases = leased_in(root.path());
        assert_eq!(leases, Leases::These(BTreeSet::from(["a".repeat(64)])));
        assert!(leases.keeps(&"a".repeat(64)) && !leases.keeps(&"b".repeat(64)));
        assert_eq!(leased(None), Leases::These(BTreeSet::new()));
        // No lock file yet: young, kept (a guest that is starting).
        let starting = root.path().join("starting");
        fs::create_dir(&starting).unwrap();
        assert_eq!(sweep(root.path(), None), 1);
        assert!(!dead_directory.exists());
        assert!(live.directory.join(LEASE).is_file(), "a live guest stays");
        assert!(starting.is_dir());
        // Its own directory is never swept, even unlocked.
        let own = live.directory.clone();
        drop(live);
        assert_eq!(sweep(root.path(), Some(&own)), 0);
        assert!(own.is_dir());
        assert_eq!(sweep(root.path(), None), 1);
        assert!(!own.exists());
    }

    /// Fail-safe: a guest that may be running and whose lease cannot be
    /// read (not written yet, or broken), or a directory without a lock
    /// file yet, keeps everything for that pass.
    #[test]
    fn an_unreadable_lease_of_a_live_guest_keeps_everything() {
        for lease in [None, Some(&b"not json"[..]), Some(&b"{\"a\":1}"[..])] {
            let root = tempfile::tempdir().unwrap();
            let live = create(root.path()).unwrap();
            if let Some(bytes) = lease {
                fs::write(live.directory.join(LEASE), bytes).unwrap();
            }
            let leases = leased_in(root.path());
            assert_eq!(leases, Leases::All, "{lease:?}");
            assert!(leases.keeps(&"c".repeat(64)));
        }
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("starting")).unwrap();
        assert_eq!(leased_in(root.path()), Leases::All);
    }

    /// A probe holds the lock it took (a sweep deletes under it); a new
    /// guest waits for a probe that lets go and keeps its directory, and
    /// gives up a directory that the probing sweep deleted meanwhile.
    #[test]
    fn a_new_guest_waits_for_a_probe_and_gives_up_a_swept_directory() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("new");
        fs::create_dir(&directory).unwrap();
        open_lock(&directory).unwrap();
        let Probe::Dead(held) = probe(&directory) else {
            panic!("an unlocked directory is dead");
        };
        assert!(
            matches!(probe(&directory), Probe::Live),
            "held while probed"
        );
        // The probe lets go without deleting: the guest gets it.
        let path = directory.clone();
        let starting = std::thread::spawn(move || create_at(&path).unwrap());
        std::thread::sleep(Duration::from_millis(200));
        drop(held);
        let guest = starting.join().unwrap().expect("kept");
        assert!(guest.directory.join(PROFILE).is_dir());
        drop(guest);

        // The probing sweep deletes it while the guest waits: given up.
        let directory = root.path().join("swept");
        fs::create_dir(&directory).unwrap();
        open_lock(&directory).unwrap();
        let Probe::Dead(held) = probe(&directory) else {
            panic!("an unlocked directory is dead");
        };
        let path = directory.clone();
        let starting = std::thread::spawn(move || create_at(&path).unwrap());
        std::thread::sleep(Duration::from_millis(200));
        fs::remove_dir_all(&directory).unwrap();
        drop(held);
        assert!(
            starting.join().unwrap().is_none(),
            "the swept one is not used"
        );
        assert!(!directory.exists(), "and not made again");
        // `create` then takes another.
        let guest = create(root.path()).unwrap();
        assert!(guest.directory.join(PROFILE).is_dir());
    }

    #[test]
    fn a_restarted_guest_adopts_its_directory_once_the_old_one_let_go() {
        let root = tempfile::tempdir().unwrap();
        let old = create(root.path()).unwrap();
        let directory = old.directory.clone();
        let other = tempfile::tempdir().unwrap();
        assert!(
            adopt_directory(root.path(), other.path()).is_err(),
            "only a guest directory"
        );
        let released = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(old);
        });
        let adopted = adopt_directory(root.path(), &directory).unwrap();
        released.join().unwrap();
        assert_eq!(adopted.directory, dunce::canonicalize(&directory).unwrap());
        assert!(matches!(probe(&directory), Probe::Live));
        drop(adopted);

        // A sweep that took it between the old process and the new one
        // deletes it under its lock: the new process does not adopt it.
        let old = create(root.path()).unwrap();
        let directory = old.directory.clone();
        drop(old);
        let Probe::Dead(held) = probe(&directory) else {
            panic!("released");
        };
        let path = directory.clone();
        let root_path = root.path().to_path_buf();
        let adopting = std::thread::spawn(move || adopt_directory(&root_path, &path).is_err());
        std::thread::sleep(Duration::from_millis(200));
        fs::remove_dir_all(&directory).unwrap();
        drop(held);
        assert!(adopting.join().unwrap(), "a swept directory is not adopted");
    }

    #[test]
    fn a_main_player_keeps_its_title() {
        assert!(!active());
        assert_eq!(decorate("a.mkv - Vestra Player"), "a.mkv - Vestra Player");
    }
}
