//! Lazy activation with the real host, headless (doc/process/rounds/
//! 2026-10-lazy-activation.md §5.1, step A1): a trivial built-in that only
//! adds a menu command is parked when idle, replayed without a process at
//! the next start, woken by `commands.execute`, and parked again. Built-ins
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
    let files = [
        (
            "manifest.json",
            serde_json::json!({"id":ID,"version":"1.0.0","apiVersion":"vestra/1",
                "entry":"main.js","capabilities":["commands"]})
            .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        (
            "main.js",
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
        ),
    ];
    let directory = root.join(ID);
    std::fs::create_dir_all(&directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    serde_json::json!({"id":ID,"version":"1.0.0","directory":ID,"digest":digest(&files)})
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
