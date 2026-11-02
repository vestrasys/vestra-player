//! `packages.install` with the real host, headless: an update of a plugin
//! the user disabled replaces its files, stays disabled and never runs (as
//! in VS Code); a first install runs. A built-in installer fixture with
//! `plugins.manage` drives the SDK (`validate`, `install`). Built-ins from a
//! `builtin.json` schema 3 under `VESTRA_BUILTIN_PLUGINS`, the development
//! runner, Node 26 on `PATH` (or `VESTRA_NODE`) and libmpv. Ignored by
//! default; `npm run ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

const INSTALLER: &str = "vestra.fixture-installer";
const DISABLED: &str = "aa.fixture-disabled";
const FRESH: &str = "aa.fixture-fresh";

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

/// Package `id` `version` with `capabilities` and `main` written to
/// `directory`; its manifest and digest.
fn package(
    directory: &Path,
    id: &str,
    version: &str,
    capabilities: &[&str],
    main: String,
) -> (serde_json::Value, String) {
    let manifest = serde_json::json!({"id":id,"version":version,"apiVersion":"vestra/1",
        "entry":"main.js","capabilities":capabilities});
    let files = [
        ("manifest.json", manifest.to_string()),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main),
    ];
    std::fs::create_dir_all(directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    (manifest, digest(&files))
}

/// A third-party plugin that logs each start with its version.
fn target(directory: &Path, id: &str, version: &str) -> (serde_json::Value, String) {
    package(
        directory,
        id,
        version,
        &[],
        format!(
            "export default function activate() {{ console.error(\"{id} start {version}\"); }}\n"
        ),
    )
}

#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn an_update_of_a_disabled_plugin_stays_disabled_and_a_first_install_runs() {
    let root = tempfile::tempdir().unwrap();
    let store = root.path().join("store");
    let updates = root.path().join("updates");
    let update = updates.join(DISABLED);
    let fresh = updates.join(FRESH);
    target(&update, DISABLED, "1.1.0");
    target(&fresh, FRESH, "1.0.0");

    // Installed 1.0.0, disabled by the user.
    let directory = store.join("packages").join("installed");
    let (manifest, digest) = target(&directory, DISABLED, "1.0.0");
    let directory = store.join("packages").join(&digest);
    std::fs::rename(store.join("packages").join("installed"), &directory).unwrap();
    let catalog = serde_json::json!({"installed":{DISABLED:{"revision":1,
            "current":{"digest":digest,"manifest":manifest,"directory":directory},
            "previous":null,
            "instance":{"guid":"00000000-0000-4000-8000-000000000001",
                "source":{"kind":"file"},"installedAt":1}}},
        "pending":{},"operations":{},"lastCommit":{},"disabled":[DISABLED]});
    std::fs::write(store.join("catalog.json"), catalog.to_string()).unwrap();

    // The installer: installs the update, then the fresh plugin, and logs
    // each outcome and state.
    let plugins = root.path().join("plugins");
    let paths = serde_json::json!([[DISABLED, update], [FRESH, fresh]]);
    let (_, installer_digest) = package(
        &plugins.join(INSTALLER),
        INSTALLER,
        "1.0.0",
        &["plugins.manage"],
        format!(
            "export default function activate(plugin) {{\n\
             setTimeout(async () => {{\n\
             for (const [id, directory] of {paths}) {{\n\
             try {{\n\
             const candidate = await plugin.packages.validate(directory);\n\
             const change = await candidate.install();\n\
             const status = await change.done;\n\
             const info = (await plugin.packages.list()).find((p) => p.id === id);\n\
             console.error(`{INSTALLER} ${{id}} ${{status}} ${{info?.state}} ${{info?.version}}`);\n\
             }} catch (error) {{ console.error(`{INSTALLER} ${{id}} error ${{error.code}} ${{error.message}}`); }}\n\
             }}\n\
             }}, 1000);\n}}\n"
        ),
    );
    std::fs::write(
        plugins.join("builtin.json"),
        serde_json::json!({"schema":3,"plugins":[{"id":INSTALLER,"version":"1.0.0",
            "directory":INSTALLER,"digest":installer_digest}]})
        .to_string(),
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_vestra-host"))
        .args(["--headless", "--silent", "--seconds", "10"])
        .env("VESTRA_BUILTIN_PLUGINS", &plugins)
        .env("VESTRA_PACKAGE_STORE", &store)
        .env("VESTRA_PLUGIN_DATA", root.path().join("data"))
        .env("VESTRA_SHARED", "0")
        .env("VESTRA_LAZY", "0")
        .env_remove("VESTRA_IDLE_MS")
        .env_remove("VESTRA_TEST_EXECUTE")
        .env_remove("VESTRA_INSTANCE_DIR")
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");

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

    let catalog: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.join("catalog.json")).unwrap()).unwrap();
    assert_eq!(catalog["disabled"], serde_json::json!([DISABLED]));
    let current = &catalog["installed"][DISABLED]["current"];
    assert_eq!(current["manifest"]["version"], "1.1.0", "files replaced");
    assert_eq!(catalog["installed"][DISABLED]["revision"], 2);
}
