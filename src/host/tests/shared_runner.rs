//! The shared runner with the real host, headless (doc/architecture/
//! README.md §2, supervisor.md §7): built-ins from a `builtin.json`
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
///
/// On-demand start is everyone's, so a built-in that holds nothing
/// live goes to standby at its first ready. These fixtures listen to the
/// player's state (resident: never in standby, supervisor.md §2), so they keep running
/// in the shared runner as they did before on-demand start became the default.
fn builtin_with(
    root: &Path,
    id: &str,
    body: &str,
    tamper: bool,
    capabilities: &[&str],
) -> serde_json::Value {
    let mut capabilities = capabilities.to_vec();
    capabilities.extend(["events", "playback.read"]);
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
                 plugin.player.onChange(() => {{}});\n\
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

/// What plugins write to their console reaches the player's stderr and the
/// plugin log beside the host log (`plugins.log`), each line once with its
/// plugin's id: a built-in in the shared runner and one running alone
/// (digest mismatch), whose lines the player prefixes itself.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn plugin_console_lines_go_to_the_plugin_log() {
    let root = plugins_root();
    let plugins = [
        builtin(
            &root.path().join("plugins"),
            "vestra.fixture-a",
            "console.warn(\"shared says hi\");",
            false,
        ),
        builtin(
            &root.path().join("plugins"),
            "vestra.fixture-c",
            "console.log(\"alone says hi\");",
            true,
        ),
    ];
    let host_log = root.path().join("logs").join("host.log");
    let stderr = run(
        &plugins,
        root.path(),
        4,
        &[("VESTRA_LOG_FILE", host_log.to_str().unwrap())],
    );
    assert!(
        stderr.contains("[vestra.fixture-a] shared says hi"),
        "{stderr}"
    );
    let plugin_log = std::fs::read_to_string(root.path().join("logs").join("plugins.log"))
        .unwrap_or_else(|error| panic!("no plugins.log ({error}); stderr:\n{stderr}"));
    for line in [
        "[vestra.fixture-a] shared says hi",
        "[vestra.fixture-c] alone says hi",
        "[vestra.fixture-a] vestra.fixture-a start 1",
    ] {
        assert_eq!(
            plugin_log.matches(line).count(),
            1,
            "{line:?} once in plugins.log:\n{plugin_log}"
        );
    }
    // The player's own lines stay in the host log.
    assert!(!plugin_log.contains("admitted"), "{plugin_log}");
}

/// Three restarts within a minute: own processes for the rest of the run.
/// `VESTRA_SHARED=0`: own processes from the start. With backoff
/// (supervisor.md §7) only the first restart in the window is immediate, the second
/// waits 3 s and the third 6 s, so the run is longer.
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
    let log = run(&plugins, root.path(), 18, &[]);
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

/// A cleanup that logs why its plugin stops (`DisposeInfo.reason` and
/// `plugin.stopReason`, SDK 1.5.54): `<id> <n> stopped for <reason> <reason>`.
fn log_stop(id: &str) -> String {
    format!(
        "plugin.onDispose((info) => console.error(\
         `{id} ${{typeof n === \"number\" ? n : 1}} stopped for ${{info?.reason}} ${{plugin.stopReason}}`));"
    )
}

/// `main` made resident (it listens to the player's state at the start of
/// `activate(plugin)`), so the plugin keeps running as it did before
/// on-demand start became the default; with the capabilities that needs.
fn resident(main: &str) -> String {
    main.replacen(
        "activate(plugin) {\n",
        "activate(plugin) {\nplugin.player.onChange(() => {});\n",
        1,
    )
}
/// The capabilities `resident` needs.
const RESIDENT_CAPABILITIES: [&str; 2] = ["events", "playback.read"];

/// An installed (third-party) plugin `id` in the package store: its own
/// process, never the shared runner; resident (`resident`).
fn install(store: &Path, catalog: &mut serde_json::Value, n: usize, id: &str, main: String) {
    let manifest = serde_json::json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1",
        "entry":"main.js","capabilities":RESIDENT_CAPABILITIES});
    let files = [
        ("manifest.json", manifest.to_string()),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", resident(&main)),
    ];
    let directory = store.join("packages").join(digest(&files));
    std::fs::create_dir_all(&directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    catalog["installed"][id] = serde_json::json!({"revision":1,
        "current":{"digest":digest(&files),"manifest":manifest,"directory":directory},
        "previous":null,
        "instance":{"guid":format!("00000000-0000-4000-8000-{n:012x}"),
            "source":{"kind":"file"},"installedAt":1}});
}

/// An unpacked package to install (`plugin.packages.validate`): `id` at
/// `version` in `directory`, resident (`resident`); its path as a
/// JavaScript string.
fn candidate(directory: &Path, id: &str, version: &str, main: String) -> String {
    std::fs::create_dir_all(directory).unwrap();
    let manifest = serde_json::json!({"id":id,"version":version,"apiVersion":"vestra/1",
        "entry":"main.js","capabilities":RESIDENT_CAPABILITIES});
    std::fs::write(directory.join("manifest.json"), manifest.to_string()).unwrap();
    std::fs::write(directory.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(directory.join("main.js"), resident(&main)).unwrap();
    serde_json::to_string(&directory.to_string_lossy()).unwrap()
}

/// Player 0.6.4: every stop the player makes carries its reason to the
/// plugin's cleanups, in the shared runner and in an own process: a
/// reload, an uncaught exception (`failed`), a disable, an uninstall, an
/// update over the running version (`upgrade`), a first install cancelled
/// while it activates (`uninstall`), and the player quitting (`quit`, both
/// runners).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn every_stop_tells_the_plugin_why() {
    let root = plugins_root();
    let plugins_dir = root.path().join("plugins");
    let store = root.path().join("store");
    let mut catalog =
        serde_json::json!({"installed":{},"pending":{},"operations":{},"lastCommit":{}});
    let up = candidate(
        &root.path().join("candidates/up"),
        "aa.fixture-up",
        "2.0.0",
        format!(
            "export default function activate(plugin) {{\n{}\n}}\n",
            log_stop("aa.fixture-up v2")
        ),
    );
    // Still activating when the manager cancels its install.
    let new = candidate(
        &root.path().join("candidates/new"),
        "aa.fixture-new",
        "1.0.0",
        format!(
            "export default async function activate(plugin) {{\n{}\n\
             await new Promise((done) => setTimeout(done, 4000));\n}}\n",
            log_stop("aa.fixture-new")
        ),
    );
    for (n, id) in [
        "aa.fixture-off",
        "aa.fixture-gone",
        "aa.fixture-stays",
        "aa.fixture-up",
    ]
    .into_iter()
    .enumerate()
    {
        install(
            &store,
            &mut catalog,
            n + 1,
            id,
            format!(
                "export default function activate(plugin) {{\n{}\n}}\n",
                log_stop(id)
            ),
        );
    }
    std::fs::write(store.join("catalog.json"), catalog.to_string()).unwrap();
    let plugins = [
        builtin(
            &plugins_dir,
            "vestra.fixture-reasons",
            &log_stop("vestra.fixture-reasons"),
            false,
        ),
        builtin(
            &plugins_dir,
            "vestra.fixture-faulty",
            &format!(
                "{}\nif (n === 1) setTimeout(() => {{ throw new Error(\"boom\"); }}, 300);",
                log_stop("vestra.fixture-faulty")
            ),
            false,
        ),
        builtin_with(
            &plugins_dir,
            "vestra.fixture-manager",
            &format!(
                "const fail = (e) => console.error(`manage failed ${{e.code}} ${{e.message}}`);\n\
                 if (n === 1) setTimeout(() => {{\n\
                   plugin.packages.reload(\"vestra.fixture-reasons\").catch(fail);\n\
                   plugin.packages.setEnabled(\"aa.fixture-off\", false).catch(fail);\n\
                   plugin.packages.uninstall(\"aa.fixture-gone\").catch(fail);\n\
                 }}, 1500);\n\
                 if (n === 1) setTimeout(async () => {{\n\
                   try {{\n\
                     const update = await (await plugin.packages.validate({up})).install();\n\
                     console.error(`update ${{await update.done}}`);\n\
                     const first = await (await plugin.packages.validate({new})).install();\n\
                     let asked = false;\n\
                     first.onChange((status) => {{\n\
                       if (status === \"activating\" && !asked) {{ asked = true; first.cancel().catch(fail); }}\n\
                     }});\n\
                     console.error(`first install ${{await first.done}}`);\n\
                   }} catch (e) {{ fail(e); }}\n\
                 }}, 3000);"
            ),
            false,
            &["plugins.manage"],
        ),
    ];
    // The plugins are resident, so they keep running.
    let log = run(&plugins, root.path(), 12, &[]);
    assert!(!log.contains("manage failed"), "{log}");
    for expected in [
        "vestra.fixture-reasons 1 stopped for reload reload",
        "vestra.fixture-reasons 2 stopped for quit quit",
        "vestra.fixture-faulty 1 stopped for failed failed",
        "aa.fixture-off 1 stopped for disable disable",
        "aa.fixture-gone 1 stopped for uninstall uninstall",
        "aa.fixture-stays 1 stopped for quit quit",
        "update committed",
        "aa.fixture-up 1 stopped for upgrade upgrade",
        "aa.fixture-up v2 1 stopped for quit quit",
        "first install cancelled",
        "aa.fixture-new 1 stopped for uninstall uninstall",
    ] {
        assert!(log.contains(expected), "{expected}: {log}");
    }
    // Each plugin stops once per instance; a stop reason is never lost to
    // `host-lost` (the pipe closing after the stop line).
    assert!(!log.contains("host-lost"), "{log}");
}
