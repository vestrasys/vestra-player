//! Guest windows with the real host (doc/architecture/guest-windows.md
//! §10, user ruling 2026-11-21): a guest leaves every file of the main
//! profile byte-identical, takes no instance lock, deletes its directory
//! at exit, and a crashed guest's directory is swept by the next start; the
//! main window can close while a guest runs, and the next launch becomes
//! the main window; a package version a running guest uses survives the
//! main player's library cleanup until that guest is gone; `app.openWindow`
//! is built-in only. Isolated profiles (`isolation.rs`), built-ins from a
//! `builtin.json` schema 3 under `VESTRA_BUILTIN_PLUGINS`, Node 26 on `PATH`
//! (or `VESTRA_NODE`), libmpv. Ignored by default; `npm run ci:local`
//! runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

mod isolation;

const FIXTURE: &str = "vestra.fixture-guest";

/// `package_stage::read_tree`'s digest over a flat fixture (as in
/// tests/kept_state.rs).
fn digest(files: &[(&str, String)]) -> String {
    let mut sorted = files.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut hasher = Sha256::new();
    hasher.update(b"vestra.package-stage/v1\0");
    for (name, text) in sorted {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update((text.len() as u64).to_le_bytes());
        hasher.update(text.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn files(id: &str, version: &str, main: &str) -> [(&'static str, String); 3] {
    [
        (
            "manifest.json",
            serde_json::json!({"id":id,"version":version,"apiVersion":"vestra/1",
                "entry":"main.js"})
            .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main.to_owned()),
    ]
}

fn write(directory: &Path, files: &[(&str, String)]) -> String {
    std::fs::create_dir_all(directory).unwrap();
    for (name, text) in files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    digest(files)
}

/// The fixture built-in: logs its data directory, writes a file there,
/// sets a secret when `VESTRA_TEST_GUEST_SECRET` is set (guest runs only:
/// a main run would write the system keychain), and asks for a new window
/// on `VESTRA_TEST_GUEST_OPEN` when that is set, unless it runs in a guest
/// (`temporary` in `VESTRA_APP_INFO`: a guest would start another guest).
/// Each outcome on stderr as `GUEST|…`.
const FIXTURE_MAIN: &str = r#"import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
export default async function activate(plugin) {
  const temporary = JSON.parse(process.env.VESTRA_APP_INFO ?? "null")?.temporary === true;
  console.error(`GUEST|temporary ${temporary}`);
  console.error(`GUEST|data ${plugin.dataDirectory}`);
  mkdirSync(plugin.dataDirectory, { recursive: true });
  writeFileSync(join(plugin.dataDirectory, "written.txt"), String(Date.now()));
  if (process.env.VESTRA_TEST_GUEST_SECRET) {
    try { await plugin.secrets.set("token", "value"); console.error("GUEST|secret set"); }
    catch (error) { console.error(`GUEST|secret ${error.code}`); }
    try { await plugin.secrets.get("token"); console.error("GUEST|secret read"); }
    catch (error) { console.error(`GUEST|secret read ${error.code}`); }
  }
  const open = process.env.VESTRA_TEST_GUEST_OPEN;
  if (open && !temporary) {
    try { const answer = await plugin.wire.request("app.openWindow", { paths: [open] }); console.error(`GUEST|opened ${answer.pid}`); }
    catch (error) { console.error(`GUEST|open ${error.code}`); }
  }
}
"#;

/// A test root: the fixture as the only built-in.
fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    let files = files(FIXTURE, "1.0.0", FIXTURE_MAIN);
    let digest = write(&plugins.join(FIXTURE), &files);
    std::fs::write(
        plugins.join("builtin.json"),
        serde_json::json!({"schema":3,"plugins":[{"id":FIXTURE,"version":"1.0.0",
            "directory":FIXTURE,"digest":digest}]})
        .to_string(),
    )
    .unwrap();
    root
}

/// The host under `root`'s isolated profile, with the fixture built-in.
fn host(root: &Path, args: &[&str], env: &[(&str, &str)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    isolation::isolate(&mut command, root);
    command
        .args(args)
        .env("VESTRA_BUILTIN_PLUGINS", root.join("plugins"))
        .env("VESTRA_LANGUAGE", "en");
    for key in [
        "VESTRA_IDLE_MS",
        "VESTRA_TEST_EXECUTE",
        "VESTRA_TEST_WIRE_TRACE",
        "VESTRA_TEST_GUEST_OPEN",
        "VESTRA_TEST_GUEST_SECRET",
        "VESTRA_TEST_OFFSCREEN",
    ] {
        command.env_remove(key);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    command
}

/// Runs to the end; its stderr (and it ended well).
fn run(mut command: Command) -> String {
    let output = command.output().unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    log
}

/// The main profile of `root` (`isolation::isolate`): the application data
/// directory, the store, the plugin data and the window state, every file
/// with its hash and every directory.
fn main_profile(root: &Path) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for top in ["appdata", "store", "data", "ui-state.json"] {
        let mut stack = vec![root.join(top)];
        while let Some(path) = stack.pop() {
            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            let name = path.strip_prefix(root).unwrap().to_string_lossy().into();
            if metadata.is_dir() {
                found.insert(name, "dir".into());
                for entry in std::fs::read_dir(&path).unwrap() {
                    stack.push(entry.unwrap().path());
                }
            } else {
                let bytes = std::fs::read(&path).unwrap();
                found.insert(name, format!("{:x}", Sha256::digest(&bytes)));
            }
        }
    }
    found
}

/// Where guests live under `root`'s isolated cache directory.
fn guests(root: &Path) -> PathBuf {
    if cfg!(target_os = "macos") {
        root.join("home/Library/Caches/Vestra/guests")
    } else if cfg!(windows) {
        root.join("local").join("Vestra").join("guests")
    } else {
        root.join("xdg-cache/vestra/guests")
    }
}

fn entries(directory: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(directory)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// Whether a live process holds `directory`'s guest lock.
fn locked(directory: &Path) -> bool {
    let Ok(file) = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.join("guest.lock"))
    else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

fn until(what: &str, seconds: u64, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The process ids a log names after `prefix`.
fn pid_after(log: &str, prefix: &str) -> Option<u32> {
    log.lines()
        .find_map(|line| line.split(prefix).nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|pid| pid.parse().ok())
}

/// Ends process `pid` (a guest this test started; never by name).
fn kill(pid: u32) {
    let status = if cfg!(windows) {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    } else {
        Command::new("kill").args(["-9", &pid.to_string()]).status()
    };
    assert!(status.unwrap().success(), "kill {pid}");
}

/// A main run fills the main profile (catalog, plugin data, window state,
/// render preference); a headless guest run then writes its plugin data,
/// is refused the secret write, reads the secret, and leaves every file of
/// the main profile byte-identical; it took no instance lock, and its
/// directory is gone after it ended. A development plugin's `app.openWindow`
/// is refused (built-in only); a headless built-in's is unsupported.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_guest_leaves_the_main_profile_byte_identical_and_cleans_up() {
    let root = root();
    let media = root.path().join("media.mkv");
    std::fs::write(&media, b"not a video").unwrap();
    let media = media.to_string_lossy().into_owned();
    // A development plugin that asks for a window: refused.
    let development = root.path().join("dev");
    write(
        &development,
        &files("fixture.dev-guest", "1.0.0", FIXTURE_MAIN),
    );
    std::fs::write(
        root.path().join("ui-state.json"),
        r#"{"window":{"x":10.0,"y":20.0,"width":800.0,"height":600.0},"appearance":{"theme":"light"},"paletteRecent":["host/stop"]}"#,
    )
    .unwrap();
    std::fs::create_dir_all(root.path().join("appdata/Vestra")).unwrap();
    std::fs::write(
        root.path().join("appdata/Vestra/render.json"),
        r#"{"preference":"auto"}"#,
    )
    .unwrap();
    let main = run({
        let mut command = host(
            root.path(),
            &["--headless", "--silent", "--seconds", "5", "--plugin"],
            &[("VESTRA_TEST_GUEST_OPEN", &media)],
        );
        command.arg(&development);
        command
    });
    assert_eq!(main.matches("GUEST|open capability").count(), 1, "{main}");
    assert_eq!(main.matches("GUEST|open unsupported").count(), 1, "{main}");
    assert!(!main.contains("GUEST|temporary true"), "{main}");
    let before = main_profile(root.path());
    assert!(
        before.keys().any(|name| name.contains("written.txt")),
        "the main run wrote its data: {before:#?}"
    );
    assert!(before.keys().any(|name| name.ends_with("catalog.json")));
    let instance = root.path().join("instance");
    let instance_text = instance.to_string_lossy().into_owned();
    let guest = run(host(
        root.path(),
        &["--guest", "--headless", "--silent", "--seconds", "5"],
        &[
            ("VESTRA_TEST_GUEST_SECRET", "1"),
            ("VESTRA_INSTANCE_DIR", &instance_text),
        ],
    ));
    let data = guest
        .lines()
        .find_map(|line| line.split("GUEST|data ").nth(1))
        .unwrap_or_else(|| panic!("the fixture ran: {guest}"));
    assert!(
        Path::new(data.trim()).starts_with(guests(root.path())),
        "its data is the guest's: {data}"
    );
    assert!(guest.contains("GUEST|temporary true"), "{guest}");
    assert!(guest.contains("GUEST|secret unavailable"), "{guest}");
    assert!(guest.contains("GUEST|secret read"), "{guest}");
    assert_eq!(main_profile(root.path()), before, "byte-identical");
    assert!(!instance.exists(), "no instance lock");
    assert_eq!(entries(&guests(root.path())), Vec::<PathBuf>::new());
}

/// An offscreen main window's built-in opens a guest (`app.openWindow`);
/// the main window quits, the guest keeps running with its lock; the next
/// launch is a main window again (it starts, not hands over); a guest
/// killed then leaves its directory, which the next start sweeps.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_guest_outlives_the_main_window_and_its_leftovers_are_swept() {
    let root = root();
    let media = root.path().join("media.mkv");
    std::fs::write(&media, b"not a video").unwrap();
    let instance = root.path().join("instance");
    let instance_text = instance.to_string_lossy().into_owned();
    let media_text = media.to_string_lossy().into_owned();
    // The guest inherits stderr: a file, so nothing waits for its end.
    let log_path = root.path().join("main.log");
    let mut command = host(
        root.path(),
        &["--silent", "--seconds", "6"],
        &[
            ("VESTRA_TEST_OFFSCREEN", "1"),
            ("VESTRA_INSTANCE_DIR", &instance_text),
            ("VESTRA_TEST_GUEST_OPEN", &media_text),
        ],
    );
    command.stderr(std::fs::File::create(&log_path).unwrap());
    assert!(command.status().unwrap().success());
    let log = std::fs::read_to_string(&log_path).unwrap();
    let pid = pid_after(&log, "GUEST|opened ").unwrap_or_else(|| panic!("{log}"));
    let result = std::panic::catch_unwind(|| {
        // Its directory, locked while it runs after the main window quit.
        until("the guest's directory", 20, || {
            entries(&guests(root.path())).len() == 1
        });
        let directory = entries(&guests(root.path())).remove(0);
        until("the guest's lock", 20, || locked(&directory));
        assert!(directory.join("profile").is_dir());
        // The next launch is the main window: it claims the lock and starts.
        let next = run(host(
            root.path(),
            &["--silent", "--seconds", "2"],
            &[
                ("VESTRA_TEST_OFFSCREEN", "1"),
                ("VESTRA_INSTANCE_DIR", &instance_text),
            ],
        ));
        assert!(
            next.contains("start: Vestra Player"),
            "not handed over: {next}"
        );
        assert!(locked(&directory), "the guest still runs");
        directory
    });
    kill(pid);
    let directory = result.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
    until("the killed guest's lock", 20, || !locked(&directory));
    assert!(directory.is_dir(), "a killed guest leaves its directory");
    run(host(
        root.path(),
        &["--headless", "--silent", "--seconds", "2"],
        &[],
    ));
    assert!(!directory.exists(), "swept by the next start");
}

/// Installs `id` with `main` into the library under `store` as a package
/// from a file (as tests/kept_state.rs `install`); its digest.
fn install(store: &Path, catalog: &mut serde_json::Value, id: &str, version: &str) -> String {
    let files = files(id, version, "export default function activate() {}\n");
    let staging = store
        .join("packages")
        .join(format!("staging-{id}-{version}"));
    let digest = write(&staging, &files);
    let directory = store.join("packages").join(&digest);
    std::fs::rename(&staging, &directory).unwrap();
    catalog["installed"][id] = serde_json::json!({"revision":1,
        "current":{"digest":digest,"manifest":serde_json::from_str::<serde_json::Value>(&files[0].1).unwrap(),"directory":directory},
        "previous":null,
        "instance":{"guid":"00000000-0000-4000-8000-000000000001",
            "source":{"kind":"file"},"installedAt":1}});
    digest
}

/// A guest runs from version 1 of a package; the main player updates it
/// to version 2 meanwhile (its catalog names 2; a main start collects what
/// the catalog does not name): version 1's directory stays while the guest
/// runs, and the first main start after the guest ended removes it.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_running_guest_keeps_the_package_version_it_runs_from() {
    let root = root();
    let store = root.path().join("store");
    std::fs::create_dir_all(store.join("packages")).unwrap();
    let mut catalog =
        serde_json::json!({"installed":{},"pending":{},"operations":{},"lastCommit":{}});
    let first = install(&store, &mut catalog, "fixture.package", "1.0.0");
    std::fs::write(store.join("catalog.json"), catalog.to_string()).unwrap();
    let old = store.join("packages").join(&first);
    let mut guest = host(
        root.path(),
        &["--guest", "--headless", "--silent", "--seconds", "20"],
        &[],
    );
    guest.stdout(Stdio::null()).stderr(Stdio::null());
    let mut guest = guest.spawn().unwrap();
    until("the guest's lease", 30, || {
        entries(&guests(root.path())).iter().any(|directory| {
            std::fs::read_to_string(directory.join("packages.json"))
                .is_ok_and(|lease| lease.contains(&first))
        })
    });
    // The update: version 2 is the installed one now.
    let second = install(&store, &mut catalog, "fixture.package", "2.0.0");
    std::fs::write(store.join("catalog.json"), catalog.to_string()).unwrap();
    let main = run(host(
        root.path(),
        &["--headless", "--silent", "--seconds", "3"],
        &[],
    ));
    assert!(old.is_dir(), "kept while the guest runs: {main}");
    assert!(store.join("packages").join(&second).is_dir());
    assert!(guest.wait().unwrap().success());
    run(host(
        root.path(),
        &["--headless", "--silent", "--seconds", "3"],
        &[],
    ));
    assert!(!old.exists(), "collected once the guest is gone");
    assert!(store.join("packages").join(&second).is_dir());
}

/// The same with windows (offscreen), where a player writes its window
/// state and its render state at quit: an offscreen main run changes the
/// main profile (the control), an offscreen guest run after it leaves every
/// file of it byte-identical and its own directory gone.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn an_offscreen_guest_leaves_the_main_profile_byte_identical() {
    let root = root();
    std::fs::write(
        root.path().join("ui-state.json"),
        r#"{"window":{"x":10.0,"y":20.0,"width":800.0,"height":600.0},"appearance":{"theme":"light"}}"#,
    )
    .unwrap();
    let instance = root.path().join("instance");
    let instance_text = instance.to_string_lossy().into_owned();
    let offscreen = [
        ("VESTRA_TEST_OFFSCREEN", "1"),
        ("VESTRA_INSTANCE_DIR", instance_text.as_str()),
    ];
    let first = main_profile(root.path());
    run(host(
        root.path(),
        &["--silent", "--seconds", "4"],
        &offscreen,
    ));
    let before = main_profile(root.path());
    assert_ne!(before, first, "an offscreen main run writes its profile");
    let guest = run(host(
        root.path(),
        &["--guest", "--silent", "--seconds", "4"],
        &offscreen,
    ));
    assert!(guest.contains("GUEST|temporary true"), "{guest}");
    assert!(guest.contains("guest: profile"), "{guest}");
    assert_eq!(main_profile(root.path()), before, "byte-identical");
    assert_eq!(entries(&guests(root.path())), Vec::<PathBuf>::new());
}
