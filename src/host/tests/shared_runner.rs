//! The shared runner with the real host, headless (doc/process/rounds/
//! 2026-10-shared-runner.md §8, §11): built-ins from a `builtin.json`
//! schema 3 under `VESTRA_BUILTIN_PLUGINS`, the development runner
//! (`src/runtime/shared-runner.ts`), Node 26 on `PATH` (or `VESTRA_NODE`)
//! and libmpv. Ignored by default; `npm run ci:local` runs it
//! (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

/// `package_stage::read_tree`'s digest (also `stage-plugins.mjs`
/// `packageDigest`), over a flat fixture.
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

/// A built-in `id` whose activation runs `body` (with `plugin` and `n`, its
/// start count this player run, kept in session storage), written to
/// `root/<id>`; returns its `builtin.json` entry.
fn builtin(root: &Path, id: &str, body: &str, tamper: bool) -> serde_json::Value {
    builtin_with(root, id, body, tamper, &[])
}

/// `builtin` with these manifest capabilities.
fn builtin_with(
    root: &Path,
    id: &str,
    body: &str,
    tamper: bool,
    capabilities: &[&str],
) -> serde_json::Value {
    let files = [
        (
            "manifest.json",
            serde_json::json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1",
                "entry":"main.js","capabilities":capabilities})
            .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        (
            "main.js",
            format!(
                "export default async function activate(plugin) {{\n\
                 const n = ((await plugin.storage.session.get(\"starts\")) ?? 0) + 1;\n\
                 await plugin.storage.session.set(\"starts\", n);\n\
                 console.error(`{id} start ${{n}} pid ${{process.pid}}`);\n\
                 {body}\n}}\n"
            ),
        ),
    ];
    let directory = root.join(id);
    std::fs::create_dir_all(&directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    let mut recorded = digest(&files);
    if tamper {
        let last = if recorded.ends_with('0') { "1" } else { "0" };
        recorded.replace_range(63.., last);
    }
    serde_json::json!({"id":id,"version":"1.0.0","directory":id,"digest":recorded})
}

/// Run the host headless for `seconds` over these built-ins; its stderr.
fn run(plugins: &[serde_json::Value], root: &Path, seconds: u32, env: &[(&str, &str)]) -> String {
    std::fs::write(
        root.join("plugins/builtin.json"),
        serde_json::json!({"schema":3,"plugins":plugins}).to_string(),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    command
        .args(["--headless", "--silent", "--seconds", &seconds.to_string()])
        .env("VESTRA_BUILTIN_PLUGINS", root.join("plugins"))
        .env("VESTRA_PACKAGE_STORE", root.join("store"))
        .env("VESTRA_PLUGIN_DATA", root.join("data"))
        .env_remove("VESTRA_SHARED")
        .env_remove("VESTRA_INSTANCE_DIR");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{stderr}");
    stderr
}

fn plugins_root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("plugins")).unwrap();
    root
}

/// Built-ins share one process; a mismatched digest runs alone; the
/// runner's exit restarts every member in a new runner, and their session
/// values survive (keyed by data key, not by process).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn built_ins_share_one_runner_and_a_restart_keeps_their_session_values() {
    let root = plugins_root();
    let plugins = [
        builtin(
            &root.path().join("plugins"),
            "vestra.fixture-a",
            // The first start ends the whole runner, as a crash would.
            "if (n === 1) setTimeout(() => process.exit(7), 300);",
            false,
        ),
        builtin(&root.path().join("plugins"), "vestra.fixture-b", "", false),
        builtin(&root.path().join("plugins"), "vestra.fixture-c", "", true),
    ];
    let log = run(&plugins, root.path(), 6, &[]);
    let pid = |line: &str| -> String {
        log.lines()
            .find(|l| l.contains(line))
            .unwrap_or_else(|| panic!("no {line:?} in\n{log}"))
            .rsplit(' ')
            .next()
            .unwrap()
            .to_owned()
    };
    assert!(log.contains("vestra.fixture-a admitted"), "{log}");
    assert!(log.contains("vestra.fixture-b admitted"), "{log}");
    assert!(
        log.contains("vestra.fixture-c runs alone: its files have digest"),
        "{log}"
    );
    // One process for a and b, another for c.
    assert_eq!(
        pid("vestra.fixture-a start 1"),
        pid("vestra.fixture-b start 1")
    );
    assert_ne!(
        pid("vestra.fixture-a start 1"),
        pid("vestra.fixture-c start 1")
    );
    // Restarted once, together, in a new runner, with their values.
    assert!(log.contains("restarting its plugins"), "{log}");
    assert_eq!(
        pid("vestra.fixture-a start 2"),
        pid("vestra.fixture-b start 2")
    );
    assert_ne!(
        pid("vestra.fixture-a start 2"),
        pid("vestra.fixture-a start 1")
    );
    assert!(
        !log.contains("vestra.fixture-c start 2"),
        "c was not restarted: {log}"
    );
    assert!(!log.contains("start 3"), "{log}");
}

/// Three restarts within a minute: own processes for the rest of the run.
/// `VESTRA_SHARED=0`: own processes from the start.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn repeated_faults_fall_back_to_own_processes_and_the_switch_turns_sharing_off() {
    let root = plugins_root();
    let plugins = [
        builtin(
            &root.path().join("plugins"),
            "vestra.fixture-crash",
            "setTimeout(() => process.exit(7), 100);",
            false,
        ),
        builtin(
            &root.path().join("plugins"),
            "vestra.fixture-calm",
            "",
            false,
        ),
    ];
    let log = run(&plugins, root.path(), 8, &[]);
    assert!(
        log.contains("every plugin runs in its own process until the player quits"),
        "{log}"
    );
    assert_eq!(log.matches("restarting its plugins").count(), 3, "{log}");
    assert!(log.contains("vestra.fixture-calm start 4"), "{log}");
    assert!(
        log.contains("vestra.fixture-calm runs alone: the shared runner failed repeatedly"),
        "{log}"
    );

    let root = plugins_root();
    let calm = builtin(
        &root.path().join("plugins"),
        "vestra.fixture-calm",
        "",
        false,
    );
    let log = run(&[calm], root.path(), 3, &[("VESTRA_SHARED", "0")]);
    assert!(
        log.contains("vestra.fixture-calm runs alone: VESTRA_SHARED=0"),
        "{log}"
    );
    assert!(!log.contains("shared runner 1"), "{log}");
}

/// The pid a fixture logged with its `n`th start.
fn pid_of(log: &str, id: &str, n: u32) -> String {
    let line = format!("{id} start {n} pid ");
    log.lines()
        .find_map(|l| l.split_once(&line).map(|(_, pid)| pid.trim().to_owned()))
        .unwrap_or_else(|| panic!("no {line:?} in\n{log}"))
}

/// A built-in that reloads `target` after `ms` (`plugins.manage`).
fn reloader(root: &Path, target: &str, ms: u32) -> serde_json::Value {
    builtin_with(
        root,
        "vestra.fixture-manager",
        &format!(
            "if (n === 1) setTimeout(() => plugin.packages.reload({target:?})\
             .catch((e) => console.error(`reload failed ${{e.code}}`)), {ms});"
        ),
        false,
        &["plugins.manage"],
    )
}

/// An uncaught exception the runner traces to one member stops that member
/// only (no restart of the others) and runs it alone when it starts again.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn an_attributed_fault_stops_only_its_plugin_and_isolates_it() {
    let root = plugins_root();
    let plugins_dir = root.path().join("plugins");
    let plugins = [
        builtin(
            &plugins_dir,
            "vestra.fixture-faulty",
            "if (n === 1) setTimeout(() => { throw new Error(\"boom\"); }, 300);",
            false,
        ),
        builtin(&plugins_dir, "vestra.fixture-calm", "", false),
        reloader(&plugins_dir, "vestra.fixture-faulty", 1500),
    ];
    let log = run(&plugins, root.path(), 5, &[]);
    let shared = pid_of(&log, "vestra.fixture-calm", 1);
    assert_eq!(pid_of(&log, "vestra.fixture-faulty", 1), shared);
    assert!(log.contains("vestra.fixture-faulty faulted"), "{log}");
    assert!(!log.contains("restarting its plugins"), "{log}");
    assert!(!log.contains("vestra.fixture-calm start 2"), "{log}");
    // Reloaded: alone, in another process; the others stay where they were.
    assert!(
        log.contains("vestra.fixture-faulty runs alone: a fault was attributed"),
        "{log}"
    );
    assert_ne!(pid_of(&log, "vestra.fixture-faulty", 2), shared);
}

/// A member that blocks the event loop: no pong for five pings, every
/// member restarts in a new runner (the spinner too: hangs are not
/// attributed), once.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_hang_restarts_every_member_in_a_new_runner() {
    let root = plugins_root();
    let plugins_dir = root.path().join("plugins");
    let plugins = [
        builtin(
            &plugins_dir,
            "vestra.fixture-spin",
            "if (n === 1) setTimeout(() => { for (;;) {} }, 500);",
            false,
        ),
        builtin(&plugins_dir, "vestra.fixture-calm", "", false),
    ];
    let log = run(
        &plugins,
        root.path(),
        8,
        &[("VESTRA_SHARED_PING_MS", "400")],
    );
    assert!(log.contains("no heartbeat for 2 s"), "{log}");
    assert_eq!(log.matches("restarting its plugins").count(), 1, "{log}");
    let first = pid_of(&log, "vestra.fixture-calm", 1);
    let second = pid_of(&log, "vestra.fixture-calm", 2);
    assert_ne!(first, second);
    assert_eq!(pid_of(&log, "vestra.fixture-spin", 2), second);
}

/// A member whose cleanup outlives the host's 300 ms window: the reload
/// starts the new instance only after the runner reports the old one gone,
/// never while the old cleanup still writes the same data directory.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_reload_waits_for_a_slow_cleanup_to_finish() {
    let root = plugins_root();
    let plugins_dir = root.path().join("plugins");
    let plugins = [
        builtin(
            &plugins_dir,
            "vestra.fixture-slow",
            "const { readFileSync, writeFileSync, appendFileSync } = await import(\"node:fs\");\n\
             const marker = `${plugin.dataDirectory}/cleanup.txt`;\n\
             let before = \"\"; try { before = readFileSync(marker, \"utf8\"); } catch {}\n\
             console.error(`vestra.fixture-slow found [${before}]`);\n\
             plugin.onDispose(async () => {\n\
               writeFileSync(marker, \"started\");\n\
               await new Promise((done) => setTimeout(done, 1500));\n\
               appendFileSync(marker, \" finished\");\n\
             });",
            false,
        ),
        reloader(&plugins_dir, "vestra.fixture-slow", 1000),
    ];
    let log = run(&plugins, root.path(), 7, &[]);
    assert!(log.contains("still cleaning up after 300 ms"), "{log}");
    pid_of(&log, "vestra.fixture-slow", 2);
    assert!(
        log.contains("vestra.fixture-slow found [started finished]"),
        "the new instance started before the old cleanup finished:\n{log}"
    );
}
