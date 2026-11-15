//! `packages.install` with the real host, headless: an update of a plugin
//! the user disabled replaces its files, stays disabled and never runs; a
//! first install runs; the optional `enabled` enables (and
//! activates) or disables explicitly; a disabled package is never started
//! when an install is restored (after a failed activation or at start-up).
//! A built-in installer fixture with `plugins.manage` drives the SDK
//! (`validate`, `install`); `enabled` goes on the wire by rewriting its
//! `packages.install` line, as an SDK that forwards it will send it.
//! Built-ins from a `builtin.json` schema 3 under `VESTRA_BUILTIN_PLUGINS`,
//! the development runner (own processes), Node 26 on `PATH` (or
//! `VESTRA_NODE`) and libmpv. Ignored by default; `npm run ci:local` runs it
//! (`--include-ignored`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
#[path = "explicit/mod.rs"]
mod explicit;
mod isolation;

const INSTALLER: &str = "vestra.fixture-installer";

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

/// Package `id` `version` with `main` (and the calls `capabilities`
/// stand for, `explicit`) written to `directory`; its manifest and digest.
fn package(
    directory: &Path,
    id: &str,
    version: &str,
    capabilities: &[&str],
    main: String,
) -> (Value, String) {
    let manifest = json!({"id":id,"version":version,"apiVersion":"vestra/1",
        "entry":"main.js"});
    let files = [
        ("manifest.json", manifest.to_string()),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", explicit::main(capabilities, &main)),
    ];
    std::fs::create_dir_all(directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    (manifest, digest(&files))
}

/// On-demand start is everyone's, so a plugin that holds nothing
/// live goes to standby at its first ready. These fixtures listen to the
/// player's state (resident: never in standby, supervisor.md §2), so they keep running
/// as they did before on-demand start became the default.
const RESIDENT_CAPABILITIES: [&str; 2] = ["events", "playback.read"];

/// A third-party plugin that logs each start with its version and the
/// session value `mark` it finds (then sets it), and each stop. `broken`
/// fails its activation. Resident (`RESIDENT_CAPABILITIES`).
fn target_main(id: &str, version: &str, broken: bool) -> String {
    let fail = if broken {
        "throw new Error(\"broken\");"
    } else {
        ""
    };
    format!(
        "export default async function activate(plugin) {{\n\
         plugin.player.onChange(() => {{}});\n\
         const mark = (await plugin.storage.session.get(\"mark\")) ?? \"none\";\n\
         console.error(`{id} start {version} mark ${{mark}}`);\n\
         await plugin.storage.session.set(\"mark\", \"set\");\n\
         plugin.onDispose((info) => console.error(`{id} stop {version} for ${{info?.reason}}`));\n\
         {fail}\n}}\n"
    )
}

/// `target_main` as a package directory to install.
fn target(directory: &Path, id: &str, version: &str, broken: bool) {
    package(
        directory,
        id,
        version,
        &RESIDENT_CAPABILITIES,
        target_main(id, version, broken),
    );
}

/// `target_main` stored in the library under `store`: its `StoredPackage`.
fn stored(store: &Path, id: &str, version: &str) -> Value {
    let staging = store
        .join("packages")
        .join(format!("staging-{id}-{version}"));
    let (manifest, digest) = package(
        &staging,
        id,
        version,
        &RESIDENT_CAPABILITIES,
        target_main(id, version, false),
    );
    let directory = store.join("packages").join(&digest);
    std::fs::rename(&staging, &directory).unwrap();
    json!({"digest":digest,"manifest":manifest,"directory":directory})
}

/// An installed record of `current`, instance number `n`.
fn installed(current: &Value, n: usize) -> Value {
    json!({"revision":1,"current":current,"previous":null,
        "instance":{"guid":format!("00000000-0000-4000-8000-{n:012x}"),
            "source":{"kind":"file"},"installedAt":1}})
}

/// One step of the installer: install `directory` (with `enabled` on the
/// wire when given), or enable `id`.
enum Step<'a> {
    Install(&'a Path, Option<bool>),
    Enable(&'a str),
}

/// A player data root whose only built-in is the installer running `steps`
/// one after the other, logging each outcome as
/// `<INSTALLER> <id> <status> <state> <version>`.
fn installer(root: &Path, steps: &[Step]) -> PathBuf {
    let steps = steps
        .iter()
        .map(|step| match step {
            Step::Install(directory, enabled) => json!({"directory":directory,"enabled":enabled}),
            Step::Enable(id) => json!({"enable":id}),
        })
        .collect::<Vec<_>>();
    let steps = Value::from(steps);
    let plugins = root.join("plugins");
    let (_, installer_digest) = package(
        &plugins.join(INSTALLER),
        INSTALLER,
        "1.0.0",
        &["plugins.manage", "events", "playback.read"],
        format!(
            "const write = process.stdout.write.bind(process.stdout);\n\
             let enabled = null;\n\
             process.stdout.write = (chunk, ...rest) => {{\n\
             if (enabled !== null && typeof chunk === \"string\") {{\n\
             try {{ const message = JSON.parse(chunk);\n\
             if (message.method === \"packages.install\") {{ message.params.enabled = enabled; chunk = `${{JSON.stringify(message)}}\\n`; }}\n\
             }} catch {{}}\n\
             }}\n\
             return write(chunk, ...rest);\n\
             }};\n\
             const sleep = (ms) => new Promise((done) => setTimeout(done, ms));\n\
             export default function activate(plugin) {{\n\
             plugin.player.onChange(() => {{}});\n\
             setTimeout(async () => {{\n\
             for (const step of {steps}) {{\n\
             if (step.enable) {{\n\
             await plugin.packages.setEnabled(step.enable, true).catch((error) => console.error(`{INSTALLER} enable error ${{error.code}}`));\n\
             await sleep(1500);\n\
             continue;\n\
             }}\n\
             let id = step.directory;\n\
             try {{\n\
             const candidate = await plugin.packages.validate(step.directory);\n\
             id = candidate.manifest.id;\n\
             enabled = step.enabled;\n\
             const change = await candidate.install();\n\
             enabled = null;\n\
             const status = await change.done;\n\
             const info = (await plugin.packages.list()).find((p) => p.id === id);\n\
             console.error(`{INSTALLER} ${{id}} ${{status}} ${{info?.state}} ${{info?.version}}`);\n\
             }} catch (error) {{ enabled = null; console.error(`{INSTALLER} ${{id}} error ${{error.code}} ${{error.message}}`); }}\n\
             }}\n\
             console.error(`{INSTALLER} done`);\n\
             }}, 2000);\n}}\n"
        ),
    );
    std::fs::write(
        plugins.join("builtin.json"),
        json!({"schema":3,"plugins":[{"id":INSTALLER,"version":"1.0.0",
            "directory":INSTALLER,"digest":installer_digest}]})
        .to_string(),
    )
    .unwrap();
    plugins
}

/// Run the host headless for `seconds` on `root` (own processes; on-demand
/// start for everyone, with resident fixtures); its stderr.
fn run(root: &Path, seconds: u32) -> String {
    let output = isolation::isolate(&mut Command::new(env!("CARGO_BIN_EXE_vestra-host")), root)
        .args(["--headless", "--silent", "--seconds", &seconds.to_string()])
        .env("VESTRA_BUILTIN_PLUGINS", root.join("plugins"))
        .env("VESTRA_PACKAGE_STORE", root.join("store"))
        .env("VESTRA_PLUGIN_DATA", root.join("data"))
        .env("VESTRA_SHARED", "0")
        .env_remove("VESTRA_IDLE_MS")
        .env_remove("VESTRA_TEST_EXECUTE")
        .env_remove("VESTRA_INSTANCE_DIR")
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    log
}

fn write_catalog(root: &Path, catalog: &Value) {
    std::fs::write(root.join("store").join("catalog.json"), catalog.to_string()).unwrap();
}

fn read_catalog(root: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(root.join("store").join("catalog.json")).unwrap())
        .unwrap()
}

#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn an_update_of_a_disabled_plugin_stays_disabled_and_a_first_install_runs() {
    const DISABLED: &str = "aa.fixture-disabled";
    const FRESH: &str = "aa.fixture-fresh";
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let update = root.join("updates").join(DISABLED);
    let fresh = root.join("updates").join(FRESH);
    target(&update, DISABLED, "1.1.0", false);
    target(&fresh, FRESH, "1.0.0", false);
    // Installed 1.0.0, disabled by the user.
    let current = stored(&root.join("store"), DISABLED, "1.0.0");
    write_catalog(
        root,
        &json!({"installed":{DISABLED:installed(&current, 1)},
            "pending":{},"operations":{},"lastCommit":{},"disabled":[DISABLED]}),
    );
    installer(
        root,
        &[Step::Install(&update, None), Step::Install(&fresh, None)],
    );
    let log = run(root, 12);
    assert!(log.contains(&format!("{INSTALLER} done")), "{log}");

    // Updated without running: the commit installed its files only.
    assert!(
        log.contains(&format!("{INSTALLER} {DISABLED} committed disabled 1.1.0")),
        "{log}"
    );
    assert!(!log.contains(&format!("{DISABLED} start")), "{log}");
    // A first install runs.
    assert!(
        log.contains(&format!("{INSTALLER} {FRESH} committed running 1.0.0")),
        "{log}"
    );
    assert!(log.contains(&format!("{FRESH} start 1.0.0")), "{log}");

    let catalog = read_catalog(root);
    assert_eq!(catalog["disabled"], json!([DISABLED]));
    let current = &catalog["installed"][DISABLED]["current"];
    assert_eq!(current["manifest"]["version"], "1.1.0", "files replaced");
    assert_eq!(catalog["installed"][DISABLED]["revision"], 2);
}

/// `enabled: true` (the store's fix for a revoked version it disabled):
/// the fix is activated and enabled. When the fix fails its activation,
/// the old (revoked, disabled) version is restored without being started.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn enabled_true_activates_a_disabled_package_and_a_failed_one_restores_it_stopped() {
    const FIXED: &str = "aa.fixture-fixed";
    const BROKEN: &str = "aa.fixture-broken";
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let fix = root.join("updates").join(FIXED);
    let broken = root.join("updates").join(BROKEN);
    target(&fix, FIXED, "1.1.0", false);
    target(&broken, BROKEN, "1.1.0", true);
    let store = root.join("store");
    let fixed_old = stored(&store, FIXED, "1.0.0");
    let broken_old = stored(&store, BROKEN, "1.0.0");
    write_catalog(
        root,
        &json!({"installed":{FIXED:installed(&fixed_old, 1),BROKEN:installed(&broken_old, 2)},
            "pending":{},"operations":{},"lastCommit":{},"disabled":[BROKEN, FIXED]}),
    );
    installer(
        root,
        &[
            Step::Install(&fix, Some(true)),
            Step::Install(&broken, Some(true)),
        ],
    );
    let log = run(root, 14);
    assert!(log.contains(&format!("{INSTALLER} done")), "{log}");

    assert!(log.contains(&format!("{FIXED} start 1.1.0")), "{log}");
    assert!(
        log.contains(&format!("{INSTALLER} {FIXED} committed running 1.1.0")),
        "{log}"
    );
    // The fix was activated and failed: the revoked version comes back as
    // it was, disabled, and does not run.
    assert!(log.contains(&format!("{BROKEN} start 1.1.0")), "{log}");
    assert!(
        log.contains(&format!("{INSTALLER} {BROKEN} failed disabled 1.0.0")),
        "{log}"
    );
    assert!(!log.contains(&format!("{BROKEN} start 1.0.0")), "{log}");
    assert!(!log.contains(&format!("{FIXED} start 1.0.0")), "{log}");

    let catalog = read_catalog(root);
    assert_eq!(catalog["disabled"], json!([BROKEN]));
    assert_eq!(
        catalog["installed"][BROKEN]["current"]["manifest"]["version"],
        "1.0.0"
    );
    assert_eq!(
        catalog["installed"][FIXED]["current"]["manifest"]["version"],
        "1.1.0"
    );
    assert_eq!(catalog["pending"], json!({}));
}

/// `enabled: false` on a running plugin: the running version stops, the
/// update is committed disabled without running, and its session values
/// go (as with `setEnabled(false)`): enabled again, the new version finds
/// none (an ordinary upgrade keeps them).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn enabled_false_stops_a_running_plugin_and_clears_its_session_values() {
    const RUNNING: &str = "aa.fixture-running";
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let update = root.join("updates").join(RUNNING);
    target(&update, RUNNING, "1.1.0", false);
    let current = stored(&root.join("store"), RUNNING, "1.0.0");
    write_catalog(
        root,
        &json!({"installed":{RUNNING:installed(&current, 1)},
            "pending":{},"operations":{},"lastCommit":{}}),
    );
    installer(
        root,
        &[Step::Install(&update, Some(false)), Step::Enable(RUNNING)],
    );
    let log = run(root, 14);
    assert!(log.contains(&format!("{INSTALLER} done")), "{log}");

    let started = log
        .find(&format!("{RUNNING} start 1.0.0 mark none"))
        .unwrap_or_else(|| panic!("{log}"));
    // Switched off by the install: `disable`, not `upgrade` (player 0.6.4).
    let stopped = log
        .find(&format!("{RUNNING} stop 1.0.0 for disable"))
        .unwrap_or_else(|| panic!("{log}"));
    let committed = log
        .find(&format!("{INSTALLER} {RUNNING} committed disabled 1.1.0"))
        .unwrap_or_else(|| panic!("{log}"));
    let enabled = log
        .find(&format!("{RUNNING} start 1.1.0 mark none"))
        .unwrap_or_else(|| panic!("session values kept: {log}"));
    assert!(
        started < stopped && stopped < committed && committed < enabled,
        "{log}"
    );
    assert_eq!(
        log.matches(&format!("{RUNNING} start 1.1.0")).count(),
        1,
        "{log}"
    );
    assert_eq!(read_catalog(root)["disabled"], Value::Null, "enabled again");
}

/// Start-up recovery of an install the player did not commit: a package
/// that was enabled gets its old version back and running; one that was
/// disabled gets it back stopped.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn start_up_recovery_does_not_start_a_disabled_package() {
    const DISABLED: &str = "aa.fixture-disabled";
    const ENABLED: &str = "aa.fixture-enabled";
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let store = root.join("store");
    let mut catalog = json!({"installed":{},"pending":{},"operations":{},"lastCommit":{},
        "disabled":[DISABLED]});
    for (n, id) in [DISABLED, ENABLED].into_iter().enumerate() {
        let old = stored(&store, id, "1.0.0");
        let new = stored(&store, id, "1.1.0");
        let before = installed(&old, n + 1);
        catalog["pending"][id] =
            json!({"id":id,"operationId":format!("op-{id}"),"before":before,"target":new});
        catalog["installed"][id] = before;
    }
    write_catalog(root, &catalog);
    installer(root, &[]);
    let log = run(root, 6);

    assert!(log.contains(&format!("{ENABLED} start 1.0.0")), "{log}");
    assert!(!log.contains(&format!("{DISABLED} start")), "{log}");
    assert!(!log.contains(&format!("{ENABLED} start 1.1.0")), "{log}");
    let catalog = read_catalog(root);
    assert_eq!(catalog["pending"], json!({}));
    assert_eq!(catalog["disabled"], json!([DISABLED]));
    for id in [DISABLED, ENABLED] {
        assert_eq!(
            catalog["installed"][id]["current"]["manifest"]["version"],
            "1.0.0"
        );
        assert_eq!(
            catalog["operations"][format!("op-{id}")]["status"],
            "failed"
        );
    }
}
