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

/// A new guest directory under `root`, locked.
fn create(root: &Path) -> Result<Guest> {
    let directory = root.join(uuid::Uuid::new_v4().simple().to_string());
    fs::create_dir(&directory).with_context(|| format!("{}", directory.display()))?;
    let lock = open_lock(&directory)?;
    lock.try_lock()
        .map_err(|error| anyhow::anyhow!("{}: {error}", directory.display()))?;
    fs::create_dir(directory.join(PROFILE))?;
    Ok(Guest {
        directory,
        _lock: lock,
    })
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
    let deadline = Instant::now() + ADOPT_WAIT;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(error) => anyhow::bail!("{error}"),
        }
    }
    // A sweep may have taken it meanwhile.
    anyhow::ensure!(canonical.join(PROFILE).is_dir(), "its profile is gone");
    Ok(Guest {
        directory: canonical,
        _lock: lock,
    })
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

/// The package digests the live guests under `root` run from: the leases
/// of the directories whose lock is held. A directory whose lock this
/// process can take belongs to no running guest; one whose lock cannot
/// be judged counts as live.
pub fn leased_in(root: &Path) -> BTreeSet<String> {
    let mut digests = BTreeSet::new();
    let Ok(entries) = fs::read_dir(root) else {
        return digests;
    };
    for entry in entries.flatten() {
        let directory = entry.path();
        if !directory.is_dir() || alive(&directory) == Some(false) {
            continue;
        }
        if let Ok(bytes) = fs::read(directory.join(LEASE))
            && let Ok(listed) = serde_json::from_slice::<BTreeSet<String>>(&bytes)
        {
            digests.extend(listed);
        }
    }
    digests
}

/// `leased_in` under this user's guests directory.
pub fn leased() -> BTreeSet<String> {
    root().map(|root| leased_in(&root)).unwrap_or_default()
}

/// Whether a guest holds `directory`'s lock: `Some(false)` when this
/// process could take it (released again at once), `None` without a lock
/// file or when it cannot be opened.
fn alive(directory: &Path) -> Option<bool> {
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join(LOCK))
        .ok()?;
    match lock.try_lock() {
        Ok(()) => Some(false),
        Err(TryLockError::WouldBlock) => Some(true),
        Err(TryLockError::Error(_)) => None,
    }
}

/// Delete the directories under `root` no guest holds (but `own`): those
/// whose lock can be taken, and those without a lock file that nothing
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
        let dead = match alive(&directory) {
            Some(alive) => !alive,
            None => {
                !directory.join(LOCK).exists()
                    && fs::metadata(&directory)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age >= UNLOCKED_KEPT)
            }
        };
        if dead {
            match fs::remove_dir_all(&directory) {
                Ok(()) => removed += 1,
                Err(error) => diag!(
                    "guest: {} left for a later start ({error})",
                    directory.display()
                ),
            }
        }
    }
    if removed > 0 {
        diag!("guest: removed {removed} temporary window folder(s) left behind");
    }
    removed
}

/// Start a guest on `paths` (`app.openWindow`): `vestra-host --guest
/// [--silent] -- <paths>`; its process id. It inherits this player's
/// environment (the test switches that name the main profile's files are
/// ignored by a guest, §1 of the design).
pub fn spawn(paths: &[PathBuf]) -> io::Result<u32> {
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(exe);
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
        let live = create(root.path()).unwrap();
        write_lease(&live.directory, &BTreeSet::from(["a".repeat(64)])).unwrap();
        let dead = create(root.path()).unwrap();
        write_lease(&dead.directory, &BTreeSet::from(["b".repeat(64)])).unwrap();
        let dead_directory = dead.directory.clone();
        drop(dead);
        // No lock file yet: young, kept (a guest that is starting).
        let starting = root.path().join("starting");
        fs::create_dir(&starting).unwrap();
        assert_eq!(
            leased_in(root.path()),
            BTreeSet::from(["a".repeat(64)]),
            "only the live guest's lease"
        );
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
        assert_eq!(alive(&directory), Some(true));
    }

    #[test]
    fn a_main_player_keeps_its_title() {
        assert!(!active());
        assert_eq!(decorate("a.mkv - Vestra Player"), "a.mkv - Vestra Player");
    }
}
