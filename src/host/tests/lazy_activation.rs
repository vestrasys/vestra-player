//! Lazy activation with the real host, headless (doc/process/rounds/
//! 2026-10-lazy-activation.md §5.1, step A1): a trivial built-in that only
//! adds a menu command is parked when idle, replayed without a process at
//! the next start, woken by `commands.execute`, and parked again; step B1:
//! a plugin with only the SDK's mirror subscriptions is parked, one with a
//! listener is resident and launched, first runs go eight at a time. Built-ins
//! from a `builtin.json` schema 3 under `VESTRA_BUILTIN_PLUGINS`, the
//! development runner, Node 26 on `PATH` (or `VESTRA_NODE`) and libmpv.
//! Ignored by default; `npm run ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

const ID: &str = "vestra.fixture-lazy";

/// `package_stage::read_tree`'s digest over a flat fixture (as in
/// tests/shared_runner.rs).
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

/// The fixture: one command, `Tools ▸ Hello`, that logs which start of
/// this player run it ran in. Its `builtin.json` entry.
fn fixture(root: &Path) -> serde_json::Value {
    package(
        root,
        ID,
        &["commands"],
        format!(
            "export default async function activate(plugin) {{\n\
             const n = ((await plugin.storage.session.get(\"starts\")) ?? 0) + 1;\n\
             await plugin.storage.session.set(\"starts\", n);\n\
             console.error(`{ID} start ${{n}}`);\n\
             plugin.commands.add({{ id: \"hello\", path: [\"Tools\", \"Hello\"],\n\
             run: () => console.error(`{ID} hello in start ${{n}}`) }});\n\
             plugin.onDispose(() => new Promise((done) => setTimeout(() => {{\n\
             console.error(`{ID} disposed ${{n}}`); done(); }}, 1200)));\n}}\n"
        ),
    )
}

/// A built-in package `id` with `capabilities` and `main.js`; its
/// `builtin.json` entry.
fn package(root: &Path, id: &str, capabilities: &[&str], main: String) -> serde_json::Value {
    let files = [
        (
            "manifest.json",
            serde_json::json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1",
                "entry":"main.js","capabilities":capabilities})
            .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main),
    ];
    let directory = root.join(id);
    std::fs::create_dir_all(&directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    serde_json::json!({"id":id,"version":"1.0.0","directory":id,"digest":digest(&files)})
}

/// A player data root whose built-ins are `entries`.
fn root_with(entries: impl Fn(&Path) -> Vec<serde_json::Value>) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    std::fs::create_dir(&plugins).unwrap();
    let entries = entries(&plugins);
    std::fs::write(
        plugins.join("builtin.json"),
        serde_json::json!({"schema":3,"plugins":entries}).to_string(),
    )
    .unwrap();
    root
}

/// The recordings under `root`, as JSON.
fn recordings(root: &Path) -> Vec<serde_json::Value> {
    std::fs::read_dir(root.join("store").join("activation"))
        .map(|entries| {
            entries
                .filter_map(|entry| std::fs::read(entry.ok()?.path()).ok())
                .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Run the host headless for `seconds`; its stderr.
fn run(root: &Path, seconds: u32, env: &[(&str, &str)]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    command
        .args(["--headless", "--silent", "--seconds", &seconds.to_string()])
        .env("VESTRA_BUILTIN_PLUGINS", root.join("plugins"))
        .env("VESTRA_PACKAGE_STORE", root.join("store"))
        .env("VESTRA_PLUGIN_DATA", root.join("data"))
        .env_remove("VESTRA_SHARED")
        .env_remove("VESTRA_LAZY")
        .env_remove("VESTRA_IDLE_MS")
        .env_remove("VESTRA_TEST_EXECUTE")
        .env_remove("VESTRA_INSTANCE_DIR");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{stderr}");
    stderr
}

fn count(log: &str, text: &str) -> usize {
    log.matches(text).count()
}

/// First run: started (no recording), parked once idle, recorded. Second
/// run: replayed without a process, woken by its command, which runs, and
/// parked again. `VESTRA_LAZY=0`: started at launch as before, even with a
/// recording. Both with the shared runner and with own processes.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn an_idle_plugin_is_parked_replayed_woken_by_its_command_and_parked_again() {
    for shared in ["1", "0"] {
        let root = tempfile::tempdir().unwrap();
        let plugins = root.path().join("plugins");
        std::fs::create_dir(&plugins).unwrap();
        let entry = fixture(&plugins);
        std::fs::write(
            plugins.join("builtin.json"),
            serde_json::json!({"schema":3,"plugins":[entry]}).to_string(),
        )
        .unwrap();
        let lazy = [
            ("VESTRA_SHARED", shared),
            ("VESTRA_LAZY", "1"),
            ("VESTRA_IDLE_MS", "500"),
        ];

        let first = run(root.path(), 5, &lazy);
        assert_eq!(count(&first, &format!("{ID} start 1")), 1, "{first}");
        assert!(!first.contains("replayed"), "{first}");
        assert!(
            first.contains(&format!("lazy: {ID} parked (1 recorded calls)")),
            "{first}"
        );
        let recordings = root.path().join("store").join("activation");
        let files = std::fs::read_dir(&recordings).unwrap().count();
        assert_eq!(files, 1, "one recording in {}", recordings.display());

        // Woken at 2.5 s; parked; woken again 100 ms after that park, while
        // its first process still runs its 1.2 s `onDispose`.
        let mut woken = lazy.to_vec();
        woken.push((
            "VESTRA_TEST_EXECUTE",
            "vestra.fixture-lazy/hello@2500,vestra.fixture-lazy/hello@park+100",
        ));
        let second = run(root.path(), 11, &woken);
        assert!(
            second.contains(&format!("lazy: {ID} replayed (1 calls)")),
            "{second}"
        );
        let replayed = second.find("replayed").unwrap();
        let execute = second.find("VESTRA_TEST_EXECUTE").unwrap();
        // Not started before its first use.
        let started = second
            .find(&format!("{ID} start 1"))
            .unwrap_or_else(|| panic!("woken: {second}"));
        assert!(replayed < execute && execute < started, "{second}");
        assert!(
            second.contains(&format!("lazy: {ID} woken by commands.execute")),
            "{second}"
        );
        assert!(
            second.contains(&format!("{ID} hello in start 1")),
            "the command ran once woken: {second}"
        );
        let ran = second.find("hello in start 1").unwrap();
        let parked = second
            .find(&format!("lazy: {ID} parked"))
            .unwrap_or_else(|| panic!("parked again: {second}"));
        assert!(ran < parked, "{second}");
        // The second wake waits for the parked process: one instance at a
        // time on its data directory (as reload and install wait).
        assert!(
            second.contains(&format!("lazy: {ID} waits for its stopped process")),
            "{second}"
        );
        let disposed = second
            .find(&format!("{ID} disposed 1"))
            .unwrap_or_else(|| panic!("disposed: {second}"));
        let again = second
            .find(&format!("{ID} start 2"))
            .unwrap_or_else(|| panic!("started again: {second}"));
        assert!(parked < disposed && disposed < again, "{second}");
        // Session storage outlives the process: its count went on.
        assert!(
            second.contains(&format!("{ID} hello in start 2")),
            "{second}"
        );
        assert_eq!(count(&second, &format!("{ID} start")), 2, "{second}");
        assert!(
            count(&second, &format!("lazy: {ID} parked")) >= 2,
            "parked twice: {second}"
        );

        let eager = run(
            root.path(),
            3,
            &[("VESTRA_SHARED", shared), ("VESTRA_LAZY", "0")],
        );
        assert!(eager.contains(&format!("{ID} start 1")), "{eager}");
        assert!(!eager.contains("lazy:"), "{eager}");
    }
}

const MIRROR: &str = "vestra.fixture-mirror";
const LISTENER: &str = "vestra.fixture-listener";

/// Step B1 (§3.1, §3.4): a plugin whose event subscriptions are all the
/// SDK's mirrors (`plugin.player.state`) is classified lazy at its first
/// ready and parked at once, replayed at the next start and woken by its
/// command, which reads the player's state; a plugin with a listener
/// (`player.onChange`) is resident, recorded so at its ready, and started
/// at launch the next time instead of being replayed.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn mirror_subscriptions_park_and_wake_a_listener_keeps_a_plugin_resident() {
    for shared in ["1", "0"] {
        let root = root_with(|plugins| {
            vec![
                package(
                    plugins,
                    MIRROR,
                    &["commands", "events", "playback.read"],
                    format!(
                        "export default async function activate(plugin) {{\n\
                         console.error(`{MIRROR} start`);\n\
                         plugin.commands.add({{ id: \"state\", path: [\"Tools\", \"State\"],\n\
                         run: () => console.error(`{MIRROR} state ${{plugin.player.state.phase}}`) }});\n}}\n"
                    ),
                ),
                package(
                    plugins,
                    LISTENER,
                    &["commands", "events", "playback.read"],
                    format!(
                        "export default async function activate(plugin) {{\n\
                         console.error(`{LISTENER} start`);\n\
                         plugin.player.onChange(() => {{}});\n\
                         plugin.commands.add({{ id: \"hello\", path: [\"Tools\", \"Hello\"], run: () => {{}} }});\n}}\n"
                    ),
                ),
            ]
        });
        let lazy = [
            ("VESTRA_SHARED", shared),
            ("VESTRA_LAZY", "1"),
            ("VESTRA_IDLE_MS", "500"),
        ];
        let first = run(root.path(), 5, &lazy);
        // First runs: the mirror plugin is parked at once, without waiting
        // for an idle time; the listener stays.
        for expected in [
            format!("lazy: {MIRROR} starts for its first run"),
            format!("lazy: {MIRROR} classified lazy"),
            format!("lazy: {MIRROR} parked (1 recorded calls)"),
            format!("lazy: {LISTENER} classified resident: an event listener"),
        ] {
            assert!(first.contains(&expected), "{expected}: {first}");
        }
        assert!(
            !first.contains(&format!("lazy: {LISTENER} parked")),
            "{first}"
        );
        let recorded = recordings(root.path());
        assert_eq!(recorded.len(), 2, "{recorded:?}");
        let resident = recorded
            .iter()
            .filter(|r| r["residentAtReady"] == true && r["calls"] == serde_json::json!([]))
            .count();
        assert_eq!(resident, 1, "one resident recording: {recorded:?}");

        let mut woken = lazy.to_vec();
        woken.push(("VESTRA_TEST_EXECUTE", "vestra.fixture-mirror/state@2500"));
        let second = run(root.path(), 6, &woken);
        for expected in [
            format!("lazy: {MIRROR} replayed (1 calls)"),
            format!("lazy: {LISTENER} starts: resident at its last ready"),
            format!("{LISTENER} start"),
            format!("lazy: {MIRROR} woken by commands.execute"),
            // The SDK read the player's state when it started.
            format!("{MIRROR} state empty"),
        ] {
            assert!(second.contains(&expected), "{expected}: {second}");
        }
        assert!(
            !second.contains(&format!("lazy: {LISTENER} replayed")),
            "{second}"
        );
        let execute = second.find("VESTRA_TEST_EXECUTE").unwrap();
        let started = second.find(&format!("{MIRROR} start")).unwrap();
        assert!(execute < started, "not started before its use: {second}");
        let ran = second.find(&format!("{MIRROR} state empty")).unwrap();
        let parked = second
            .rfind(&format!("lazy: {MIRROR} parked"))
            .unwrap_or_else(|| panic!("parked again: {second}"));
        assert!(ran < parked, "{second}");
    }
}

/// §2.4: on a fresh profile at most eight first runs are in flight; a lazy
/// one frees its place once parked and gone, and the queue goes on.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn first_runs_start_eight_at_a_time() {
    let ids = (0..10)
        .map(|n| format!("vestra.fixture-first-{n}"))
        .collect::<Vec<_>>();
    let root = root_with(|plugins| {
        ids.iter()
            .map(|id| {
                package(
                    plugins,
                    id,
                    &["commands"],
                    format!(
                        "export default function activate(plugin) {{\n\
                         plugin.commands.add({{ id: \"go\", path: [\"Tools\", \"{id}\"], run: () => {{}} }});\n}}\n"
                    ),
                )
            })
            .collect()
    });
    let log = run(
        root.path(),
        8,
        &[
            ("VESTRA_SHARED", "1"),
            ("VESTRA_LAZY", "1"),
            ("VESTRA_IDLE_MS", "60000"),
        ],
    );
    let starts = log
        .match_indices("starts for its first run")
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 10, "{log}");
    let first_park = log
        .find("parked (1 recorded calls)")
        .unwrap_or_else(|| panic!("{log}"));
    assert!(starts[7] < first_park && first_park < starts[8], "{log}");
    for id in &ids {
        let parked = format!("lazy: {id} parked (1 recorded calls)");
        assert!(log.contains(&parked), "{parked}: {log}");
    }
    assert_eq!(recordings(root.path()).len(), 10);
}
