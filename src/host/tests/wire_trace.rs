//! The wire trace on controlled plugins (core thread plan S1.1; the
//! coordinator's ruling of 2026-11-15 on its done criterion): the host runs
//! offscreen twice with `VESTRA_TEST_WIRE_TRACE` and the same plugins, and
//! `src/scripts/verify-wire.mjs` must find the two traces **exactly** equal
//! (strict mode) and each free of single-trace violations (`check`). This is
//! the gate for S2 (zero difference) and, with `--allow-reorders`, for S3a.
//!
//! The plugins do the same work whatever the timing:
//!
//! - one makes sequential awaited calls;
//! - one sends a UI-bound request (`ui.mount` of a panel) and a
//!   UI-independent one without awaiting either, then sets the video area
//!   (UI-bound, with the SDK's `window.state` read after it);
//! - the stall test's three, with counts instead of clocks: ten `source.state`
//!   requests 50 ms apart without awaiting, a `time` trigger 1.5 s after its
//!   start, and a built-in used five times and then parked when quiet.
//!
//! Needs Node 26 on `PATH` (or `VESTRA_NODE`), the development runner and
//! libmpv. Ignored by default; `npm run ci:local` runs it
//! (`--include-ignored`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

/// `package_stage::read_tree`'s digest over a fixture (as in
/// tests/lazy_activation.rs).
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

/// A plugin package `id` with `main.js` and `extra` files under `root`; its
/// `builtin.json` entry.
fn package(root: &Path, id: &str, main: &str, extra: &[(&str, &str)]) -> Value {
    let mut files = vec![
        (
            "manifest.json",
            json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1","entry":"main.js"})
                .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main.to_owned()),
    ];
    files.extend(extra.iter().map(|(name, text)| (*name, (*text).to_owned())));
    let directory = root.join(id);
    for (name, text) in &files {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    json!({"id":id,"version":"1.0.0","directory":id,"digest":digest(&files)})
}

const SEQUENTIAL: &str = "export default async function activate(plugin) {\n\
     await plugin.player.read();\n\
     await plugin.storage.session.set(\"k\", 1);\n\
     await plugin.storage.session.get(\"k\");\n\
     plugin.commands.add({ id: \"hello\", path: [\"Tools\", \"Wire check\"], run: () => {} });\n\
     for (let i = 0; i < 5; i++) await plugin.player.refresh();\n\
   }\n";

const INTERLEAVE: &str = "export default async function activate(plugin) {\n\
     const panel = plugin.panels.mount(\"sidebar\", \"ui/list.slint\", { title: \"Wire\" });\n\
     const read = plugin.storage.session.get(\"k\");\n\
     await Promise.all([panel.ready, read]);\n\
     await plugin.window.setVideoArea({ width: 640, height: 360 }).catch(() => {});\n\
   }\n";

const PANEL: &str = "export global Bridge {\n    in property <bool> tabbed;\n}\n\
     export component List inherits Rectangle {\n    background: #1a1821;\n\
     Text { text: \"wire\"; color: white; }\n}\n";

const POLLER: &str = "export default function activate(plugin) {\n\
     let left = 10;\n\
     const timer = setInterval(() => {\n\
       if (--left < 0) return clearInterval(timer);\n\
       plugin.player.refresh().catch(() => {});\n\
     }, 50);\n\
   }\n";

const TRIGGER: &str = "export default function activate(plugin) {\n\
     plugin.trigger(\"soon\")\n\
       .firesOn(plugin.when.time({ min: Date.now() + 1500 }))\n\
       .enter(() => plugin.storage.session.set(\"fired\", true));\n\
   }\n";

const PARKER: &str = "export default function activate(plugin) {\n\
     void (async () => {\n\
       for (let i = 0; i < 5; i++) {\n\
         await plugin.storage.session.get(\"k\");\n\
         await new Promise((done) => setTimeout(done, 100));\n\
       }\n\
     })();\n\
   }\n";

/// The fixtures under `root`: the built-ins' directory and the development
/// plugins.
fn fixtures(root: &Path) -> (PathBuf, Vec<PathBuf>) {
    let builtins = root.join("builtins");
    let development = root.join("development");
    std::fs::create_dir_all(&builtins).unwrap();
    let parker = package(&builtins, "vestra.fixture-wire-park", PARKER, &[]);
    std::fs::write(
        builtins.join("builtin.json"),
        json!({"schema":3,"plugins":[parker]}).to_string(),
    )
    .unwrap();
    let mut plugins = Vec::new();
    for (id, main, extra) in [
        ("test.wire-sequential", SEQUENTIAL, &[][..]),
        (
            "test.wire-interleave",
            INTERLEAVE,
            &[("ui/list.slint", PANEL)][..],
        ),
        ("test.wire-poll", POLLER, &[][..]),
        ("test.wire-trigger", TRIGGER, &[][..]),
    ] {
        package(&development, id, main, extra);
        plugins.push(development.join(id));
    }
    (builtins, plugins)
}

/// One offscreen run with its own profile under `profile`; its trace.
fn run(profile: &Path, builtins: &Path, plugins: &[PathBuf]) -> PathBuf {
    let trace = profile.join("run.wire.jsonl");
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    command
        .args(["--silent", "--seconds", "8"])
        .env("VESTRA_TEST_OFFSCREEN", "1")
        .env("VESTRA_TEST_WIRE_TRACE", &trace)
        .env("VESTRA_BUILTIN_PLUGINS", builtins)
        .env("VESTRA_IDLE_MS", "1000")
        .env("VESTRA_PACKAGE_STORE", profile.join("store"))
        .env("VESTRA_PLUGIN_DATA", profile.join("data"))
        .env("VESTRA_UI_STATE", profile.join("ui-state.json"))
        .env("VESTRA_INSTANCE_DIR", profile.join("instance"))
        .env("VESTRA_SHADER_CACHE", profile.join("shader-cache"))
        .env("APPDATA", profile.join("appdata"))
        .env("LOCALAPPDATA", profile.join("local"))
        .env("VESTRA_LANGUAGE", "en");
    for key in [
        "VESTRA_SHARED",
        "VESTRA_TEST_EXECUTE",
        "VESTRA_TEST_PARK",
        "VESTRA_TEST_WAKE",
        "VESTRA_TEST_UI_STALL",
        "VESTRA_LOG_FILE",
        "VESTRA_FRAME_LOG",
    ] {
        command.env_remove(key);
    }
    for plugin in plugins {
        command.arg("--plugin").arg(plugin);
    }
    let output = command.output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("lazy: vestra.fixture-wire-park parked"),
        "the built-in parks: {stderr}"
    );
    trace
}

/// `node src/scripts/verify-wire.mjs <args>`: its exit code and output.
fn verify_wire(args: &[&std::ffi::OsStr]) -> (i32, String) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/verify-wire.mjs");
    let node = std::env::var_os("VESTRA_NODE").unwrap_or_else(|| "node".into());
    let output = Command::new(node).arg(script).args(args).output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.code().unwrap_or(-1), text)
}

/// A trace's records of `plugin`.
fn records_of(text: &str, plugin: &str) -> Vec<Value> {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["plugin"] == plugin)
        .collect()
}

/// `plugin`'s requests (by `method` when given) and how many of them
/// were answered, per process.
fn answered(text: &str, plugin: &str, method: Option<&str>) -> (usize, usize) {
    let records = records_of(text, plugin);
    let key = |record: &Value| (record["owner"].as_u64(), record["line"]["id"].as_u64());
    let requests: std::collections::BTreeSet<_> = records
        .iter()
        .filter(|r| r["dir"] == "in" && r["line"]["method"].is_string())
        .filter(|r| method.is_none_or(|m| r["line"]["method"] == m))
        .map(key)
        .collect();
    let replies = records
        .iter()
        .filter(|r| r["dir"] == "out" && r["line"].get("method").is_none())
        .filter(|r| r["line"].get("result").is_some() || r["line"].get("error").is_some())
        .filter(|r| requests.contains(&key(r)))
        .count();
    (requests.len(), replies)
}

#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn two_runs_of_controlled_plugins_have_exactly_equal_wire_traces() {
    let root = tempfile::tempdir().unwrap();
    let (builtins, plugins) = fixtures(root.path());
    let profiles = [root.path().join("a"), root.path().join("b")];
    let traces: Vec<_> = profiles
        .iter()
        .map(|profile| run(profile, &builtins, &plugins))
        .collect();
    let roots: Vec<String> = profiles
        .iter()
        .map(|profile| format!("{}=<profile>", profile.display()))
        .collect();
    for trace in &traces {
        let (code, text) = verify_wire(&["check".as_ref(), trace.as_os_str()]);
        assert_eq!(code, 0, "{text}");
        // Every plugin talked.
        let lines = std::fs::read_to_string(trace).unwrap();
        for id in [
            "test.wire-sequential",
            "test.wire-interleave",
            "test.wire-poll",
            "test.wire-trigger",
            "vestra.fixture-wire-park",
        ] {
            assert!(lines.contains(&format!("\"plugin\":\"{id}\"")), "{id}");
        }
        assert!(lines.contains("\"method\":\"ui.mount\""));
        // Counts, so that a hook that drops a line in every run fails.
        assert_eq!(
            answered(&lines, "test.wire-poll", Some("source.state")),
            (10, 10),
            "ten polls, ten replies"
        );
        let fired = records_of(&lines, "test.wire-trigger")
            .iter()
            .filter(|r| r["dir"] == "out" && r["line"]["event"] == "triggers.fired")
            .count();
        assert_eq!(fired, 1, "the trigger fired once");
        let (requests, replies) = answered(&lines, "test.wire-sequential", None);
        // read: source.state, timeline.marks; set, get; the command; five
        // refreshes; the SDK's window.state read.
        assert_eq!(requests, 11, "the sequential plugin's requests");
        assert_eq!(replies, requests, "every sequential request answered");
        let (requests, replies) = answered(&lines, "vestra.fixture-wire-park", None);
        assert_eq!(replies, requests, "every parked plugin's request answered");
        if let Some(dir) = std::env::var_os("VESTRA_TEST_LOG_DIR") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            let name = trace.parent().unwrap().file_name().unwrap();
            std::fs::copy(trace, dir.join(name).with_extension("wire.jsonl")).unwrap();
        }
    }
    for mode in [None, Some("--allow-reorders")] {
        let mut args: Vec<&std::ffi::OsStr> = vec![
            "diff".as_ref(),
            traces[0].as_os_str(),
            traces[1].as_os_str(),
        ];
        for root in &roots {
            args.push("--root".as_ref());
            args.push(root.as_ref());
        }
        if let Some(mode) = mode {
            args.push(mode.as_ref());
        }
        let (code, text) = verify_wire(&args);
        assert_eq!(code, 0, "{text}");
        assert!(text.contains("RESULT: PASS, no difference"), "{text}");
        assert!(!text.contains("ALLOWED REORDERS"), "{text}");
    }
}
