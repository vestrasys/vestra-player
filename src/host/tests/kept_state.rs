//! What the host keeps for a plugin in standby, and the state stream, with
//! the real host, headless (events-and-triggers.md §4.7, §4.8; plan steps
//! A3 and A4). A timeline layer is kept by the installed instance through
//! standby and no longer keeps its plugin running: it is still there after
//! a wake, the woken run's next write replaces it, a fault removes it, a
//! new source clears it, and a disable, uninstall, reload or upgrade
//! removes it. The cursor setting no longer keeps its plugin running.
//! `plugin.player.stream(fn)` keeps its plugin running while it is open;
//! after its `stop()` a run that holds nothing else goes to standby after
//! its idle time. Built-ins from a `builtin.json` schema 3 under
//! `VESTRA_BUILTIN_PLUGINS`, installed packages in the library, the
//! development runner, Node 26 on `PATH` (or `VESTRA_NODE`), libmpv and
//! ffmpeg (`PATH` or `VESTRA_FFMPEG`, as for ci:local). Ignored by default;
//! `npm run ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

mod isolation;

/// `package_stage::read_tree`'s digest over a flat fixture (as in
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

/// The files of package `id` `version` with `main`.
fn files(id: &str, version: &str, main: String) -> [(&'static str, String); 3] {
    [
        (
            "manifest.json",
            serde_json::json!({"id":id,"version":version,"apiVersion":"vestra/1",
                "entry":"main.js"})
            .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main),
    ]
}

/// Writes `files` to `directory`; their digest.
fn write(directory: &Path, files: &[(&str, String)]) -> String {
    std::fs::create_dir_all(directory).unwrap();
    for (name, text) in files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    digest(files)
}

/// A built-in package `id` with `main.js`; its `builtin.json` entry.
fn builtin(plugins: &Path, id: &str, main: String) -> serde_json::Value {
    let files = files(id, "1.0.0", main);
    let digest = write(&plugins.join(id), &files);
    serde_json::json!({"id":id,"version":"1.0.0","directory":id,"digest":digest})
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

/// Installs `id` in the library under `store` as a package from a file
/// (as tests/lazy_activation.rs `install`).
fn install(store: &Path, catalog: &mut serde_json::Value, n: usize, id: &str, main: String) {
    let files = files(id, "1.0.0", main);
    let staging = store.join("packages").join(format!("staging-{id}"));
    let digest = write(&staging, &files);
    let directory = store.join("packages").join(&digest);
    std::fs::rename(&staging, &directory).unwrap();
    catalog["installed"][id] = serde_json::json!({"revision":1,
        "current":{"digest":digest,"manifest":serde_json::from_str::<serde_json::Value>(&files[0].1).unwrap(),"directory":directory},
        "previous":null,
        "instance":{"guid":format!("00000000-0000-4000-8000-{n:012x}"),
            "source":{"kind":"file"},"installedAt":1}});
}

/// Run the host headless for `seconds`; its stderr.
fn run(root: &Path, seconds: u32, env: &[(&str, &str)]) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    isolation::isolate(&mut command, root);
    command
        .args(["--headless", "--silent", "--seconds", &seconds.to_string()])
        .env("VESTRA_BUILTIN_PLUGINS", root.join("plugins"))
        .env("VESTRA_PACKAGE_STORE", root.join("store"))
        .env("VESTRA_PLUGIN_DATA", root.join("data"))
        .env_remove("VESTRA_SHARED")
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

/// A generated tone (ffmpeg), as a JSON string for a fixture's source.
fn tone(directory: &Path) -> String {
    let path = directory.join("tone.wav");
    let ffmpeg = std::env::var("VESTRA_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let made = Command::new(ffmpeg)
        .args(["-v", "error", "-f", "lavfi", "-i", "sine=duration=20", "-y"])
        .arg(&path)
        .status()
        .unwrap();
    assert!(made.success());
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

/// A resident built-in that logs the merged timeline at every
/// `timeline.changed` as `<tag> layers <plugin>:<marks>,…` (`-`: none),
/// then runs `then` (JavaScript, `plugin` in scope) once activated.
fn watcher(tag: &str, then: &str) -> String {
    format!(
        "export default function activate(plugin) {{\n\
         const summary = (marks) => {{ const n = {{}};\n\
         for (const m of marks) n[m.plugin] = (n[m.plugin] ?? 0) + 1;\n\
         return Object.keys(n).sort().map((k) => `${{k}}:${{n[k]}}`).join(\",\") || \"-\"; }};\n\
         plugin.player.on(\"timeline.changed\", (d) => console.error(`{tag} layers ${{summary(d.marks)}}`));\n\
         {then}\n}}\n"
    )
}

/// A plugin with one timeline layer and commands that set it: `mark` one
/// mark, `marks2` two others, `crash` ends its process with a fault.
fn layered(id: &str) -> String {
    format!(
        "export default function activate(plugin) {{\n\
         console.error(`{id} start`);\n\
         const layer = plugin.timeline.layer();\n\
         const set = (times) => layer.set(times.map((time) => ({{ time, kind: \"marker\" }})));\n\
         plugin.commands.add({{ id: \"mark\", title: \"Mark\", run: () => set([1]) }});\n\
         plugin.commands.add({{ id: \"marks2\", title: \"Marks\", run: () => set([2, 3]) }});\n\
         plugin.commands.add({{ id: \"crash\", title: \"Crash\", run: () => process.exit(3) }});\n}}\n"
    )
}

/// The `<tag> layers …` lines of `log`, in order, from byte `from` on.
fn layers<'a>(log: &'a str, tag: &str, from: usize) -> Vec<&'a str> {
    let prefix = format!("{tag} layers ");
    log[from..]
        .lines()
        .filter_map(|line| line.split(&prefix).nth(1))
        .map(str::trim_end)
        .collect()
}

fn at(log: &str, text: &str) -> usize {
    log.find(text)
        .unwrap_or_else(|| panic!("{text} not found: {log}"))
}

const WATCH: &str = "vestra.fixture-watch";
const LAYER: &str = "vestra.fixture-layer";
const OTHER: &str = "vestra.fixture-layer-b";
const CURSOR: &str = "vestra.fixture-cursor";

/// Built-ins in their own processes (a fault ends one process only). The
/// layer plugin is parked at its first run; woken by `mark` it sets one
/// mark and is parked again (a layer holds nothing that keeps it running),
/// with its layer kept: the watcher hears no change. Woken by `marks2`, its
/// layer is still there until its write replaces it (two marks, not three);
/// parked again; woken by `crash`, its fault removes the layer. The second
/// layer, kept while its plugin is parked, is cleared by a new source. A
/// plugin that wrote the cursor setting is parked as well.
#[test]
#[ignore = "needs Node 26, libmpv, ffmpeg and the SDK of player 0.10.0: run with --include-ignored (ci:local)"]
fn a_layer_is_kept_through_standby_replaced_after_a_wake_and_removed_at_a_fault() {
    let media = tempfile::tempdir().unwrap();
    let path = tone(media.path());
    let root = root_with(|plugins| {
        vec![
            builtin(
                plugins,
                WATCH,
                watcher(
                    WATCH,
                    &format!(
                        "setTimeout(() => plugin.player.open({path}).then(\n\
                         () => console.error(`{WATCH} opened`),\n\
                         (error) => console.error(`{WATCH} failed ${{error}}`)), 10000);"
                    ),
                ),
            ),
            builtin(plugins, LAYER, layered(LAYER)),
            builtin(plugins, OTHER, layered(OTHER)),
            builtin(
                plugins,
                CURSOR,
                format!(
                    "export default function activate(plugin) {{\n\
                     console.error(`{CURSOR} start`);\n\
                     plugin.commands.add({{ id: \"cursor\", title: \"Cursor\", run: () =>\n\
                     plugin.window.setCursorAutoHide(\"playing\").then(() => console.error(`{CURSOR} set`)) }});\n}}\n"
                ),
            ),
        ]
    });
    // Each `park+` entry is timed from the plugin's next park: the first
    // run's, then each after a wake.
    let executes = [
        format!("{LAYER}/mark@park+300"),
        format!("{LAYER}/marks2@park+300"),
        format!("{LAYER}/crash@park+300"),
        format!("{OTHER}/mark@park+300"),
        format!("{CURSOR}/cursor@park+300"),
    ]
    .join(",");
    let env = [
        ("VESTRA_SHARED", "0"),
        ("VESTRA_IDLE_MS", "500"),
        ("VESTRA_TEST_EXECUTE", executes.as_str()),
    ];
    let log = run(root.path(), 14, &env);

    // Each was parked after its write: nothing it wrote keeps it running.
    for id in [LAYER, OTHER, CURSOR] {
        assert!(
            log.matches(&format!("lazy: {id} parked")).count() >= 2,
            "{id} parked after its write: {log}"
        );
        assert!(!log.contains(&format!("lazy: {id} stays running")), "{log}");
    }
    let set = at(&log, &format!("{CURSOR} set"));
    assert!(
        log[set..].contains(&format!("lazy: {CURSOR} parked")),
        "{log}"
    );

    let mark = at(&log, &format!("lazy: VESTRA_TEST_EXECUTE {LAYER}/mark"));
    let marks2 = at(&log, &format!("lazy: VESTRA_TEST_EXECUTE {LAYER}/marks2"));
    let crash = at(&log, &format!("lazy: VESTRA_TEST_EXECUTE {LAYER}/crash"));
    let opened = at(&log, &format!("{WATCH} opened"));
    assert!(mark < marks2 && marks2 < crash && crash < opened, "{log}");
    // Parked between the writes.
    assert!(
        log[mark..marks2].contains(&format!("lazy: {LAYER} parked")),
        "{log}"
    );
    // What the watcher heard. Before the crash every change shows the
    // first layer once it appeared: kept through each park, and through
    // the wake until its write replaced it (1 mark, then 2, never 3).
    // (A change line may come before the host's line about the fault:
    // the crash command is the cut.)
    at(&log, &format!("supervisor: {LAYER} stopped unexpectedly"));
    let first = at(&log, &format!("{LAYER}:1"));
    let first = log[..first].rfind('\n').map_or(0, |n| n + 1);
    for line in layers(&log[..crash], WATCH, first) {
        assert!(
            line.contains(&format!("{LAYER}:1")) || line.contains(&format!("{LAYER}:2")),
            "kept through standby: {line}: {log}"
        );
    }
    let replaced = format!("{WATCH} layers {LAYER}:2,{OTHER}:1");
    assert!(log[marks2..crash].contains(&replaced), "{log}");
    assert!(!log.contains(&format!("{LAYER}:3")), "{log}");
    // The fault removed it; the other, parked, was kept until the new source.
    let after_crash = layers(&log, WATCH, crash);
    assert_eq!(
        after_crash.first().copied(),
        Some(format!("{OTHER}:1").as_str()),
        "{log}"
    );
    // Cleared by the new source (its change may be logged before `opened`).
    assert_eq!(after_crash.last().copied(), Some("-"), "{log}");
    assert_eq!(after_crash.len(), 2, "{log}");
}

const CONTROL: &str = "vestra.fixture-control";
const DISABLE: &str = "aa.fixture-disable";
const UNINSTALL: &str = "aa.fixture-uninstall";
const RELOAD: &str = "aa.fixture-reload";
const UPGRADE: &str = "aa.fixture-upgrade";

/// Third-party plugins, each parked after it set its layer, every layer
/// kept. Then one is disabled, one uninstalled, one reloaded, one
/// upgraded: each removes that plugin's layer only, the others stay.
#[test]
#[ignore = "needs Node 26, libmpv and the SDK of player 0.10.0: run with --include-ignored (ci:local)"]
fn a_kept_layer_goes_with_a_disable_an_uninstall_a_reload_and_an_upgrade() {
    let updates = tempfile::tempdir().unwrap();
    let update = updates.path().join(UPGRADE);
    write(&update, &files(UPGRADE, "1.1.0", layered(UPGRADE)));
    let update = serde_json::to_string(&update.to_string_lossy()).unwrap();
    let steps = format!(
        "const sleep = (ms) => new Promise((done) => setTimeout(done, ms));\n\
         const step = async (name, act) => {{\n\
         try {{ await act(); console.error(`{CONTROL} ${{name}} done`); }}\n\
         catch (error) {{ console.error(`{CONTROL} ${{name}} failed ${{error.code}} ${{error.message}}`); }}\n\
         await sleep(1500); }};\n\
         setTimeout(async () => {{\n\
         await step(\"disable\", () => plugin.packages.setEnabled(\"{DISABLE}\", false));\n\
         await step(\"uninstall\", async () => (await plugin.packages.uninstall(\"{UNINSTALL}\")).done);\n\
         await step(\"reload\", () => plugin.packages.reload(\"{RELOAD}\"));\n\
         await step(\"upgrade\", async () => (await (await plugin.packages.validate({update})).install()).done);\n\
         }}, 7000);"
    );
    let root = root_with(|plugins| vec![builtin(plugins, CONTROL, watcher(CONTROL, &steps))]);
    let store = root.path().join("store");
    let mut catalog =
        serde_json::json!({"installed":{},"pending":{},"operations":{},"lastCommit":{}});
    let targets = [DISABLE, UNINSTALL, RELOAD, UPGRADE];
    for (n, id) in targets.iter().enumerate() {
        install(&store, &mut catalog, n + 1, id, layered(id));
    }
    std::fs::write(store.join("catalog.json"), catalog.to_string()).unwrap();
    let executes = targets
        .iter()
        .map(|id| format!("{id}/mark@park+300"))
        .collect::<Vec<_>>()
        .join(",");
    let env = [
        ("VESTRA_SHARED", "1"),
        ("VESTRA_IDLE_MS", "500"),
        ("VESTRA_TEST_EXECUTE", executes.as_str()),
    ];
    let log = run(root.path(), 20, &env);
    for id in targets {
        let marked = at(&log, &format!("lazy: VESTRA_TEST_EXECUTE {id}/mark"));
        assert!(
            log[marked..].contains(&format!("lazy: {id} parked")),
            "{id} parked with its layer: {log}"
        );
    }
    let all = format!("{CONTROL} layers {DISABLE}:1,{RELOAD}:1,{UNINSTALL}:1,{UPGRADE}:1");
    let all = at(&log, &all);
    for name in ["disable", "uninstall", "reload", "upgrade"] {
        at(&log, &format!("{CONTROL} {name} done"));
    }
    // From the moment all four were parked with their layers: nothing
    // changed until the first operation, and each operation removed its
    // plugin's layer only (a change may be logged before its `done`).
    let mut heard = layers(&log, CONTROL, all);
    heard.dedup();
    let expected = [
        format!("{DISABLE}:1,{RELOAD}:1,{UNINSTALL}:1,{UPGRADE}:1"),
        format!("{RELOAD}:1,{UNINSTALL}:1,{UPGRADE}:1"),
        format!("{RELOAD}:1,{UPGRADE}:1"),
        format!("{UPGRADE}:1"),
        "-".to_owned(),
    ];
    assert_eq!(heard, expected, "{log}");
    let last = layers(&log, CONTROL, 0);
    assert_eq!(last.last().copied(), Some("-"), "{log}");
    assert!(!log.contains(" failed "), "{log}");
}

const STREAM: &str = "vestra.fixture-stream";
const AT_READY: &str = "vestra.fixture-stream-at-ready";

/// `plugin.player.stream(fn)` opened by a command after the ready keeps
/// its plugin running while it is open (the idle check says so, with
/// where the stream was opened); after `stop()` the plugin, which holds
/// nothing else, is parked after its idle time. A stream open at the ready
/// is a listener there: the plugin is classified resident.
#[test]
#[ignore = "needs Node 26, libmpv and the SDK of player 0.10.0: run with --include-ignored (ci:local)"]
fn a_stream_keeps_its_plugin_running_until_it_stops() {
    let root = root_with(|plugins| {
        vec![
            builtin(
                plugins,
                STREAM,
                format!(
                    "export default function activate(plugin) {{\n\
                     console.error(`{STREAM} start`);\n\
                     let feed;\n\
                     plugin.commands.add({{ id: \"open\", title: \"Open\", run: () => {{\n\
                     feed = plugin.player.stream((state) => console.error(`{STREAM} state ${{state.phase}}`));\n\
                     console.error(`{STREAM} opened`); }} }});\n\
                     plugin.commands.add({{ id: \"close\", title: \"Close\", run: () => {{\n\
                     feed?.stop(); console.error(`{STREAM} stopped`); }} }});\n}}\n"
                ),
            ),
            builtin(
                plugins,
                AT_READY,
                "export default function activate(plugin) {\n\
                 plugin.player.stream(() => {});\n}\n"
                    .to_owned(),
            ),
        ]
    });
    let executes = format!("{STREAM}/open@park+300,{STREAM}/close@5000");
    let env = [
        ("VESTRA_SHARED", "1"),
        ("VESTRA_IDLE_MS", "500"),
        ("VESTRA_TEST_EXECUTE", executes.as_str()),
    ];
    let log = run(root.path(), 8, &env);
    let opened = at(&log, &format!("{STREAM} opened"));
    let stopped = at(&log, &format!("{STREAM} stopped"));
    assert!(opened < stopped, "{log}");
    // The first call has the current state.
    assert!(
        log[opened..stopped].contains(&format!("{STREAM} state empty")),
        "{log}"
    );
    // Kept running while open, and the idle check says why.
    assert!(
        !log[opened..stopped].contains(&format!("lazy: {STREAM} parked")),
        "{log}"
    );
    let why = format!(
        "lazy: {STREAM} stays running: an event listener; listens to source.state: a stream opened at main.js:"
    );
    let stays = at(&log, &why);
    assert!(opened < stays && stays < stopped, "{log}");
    // Parked after its idle time once it stopped.
    assert!(
        log[stopped..].contains(&format!("lazy: {STREAM} parked")),
        "{log}"
    );
    let resident = format!(
        "lazy: {AT_READY} classified resident: an event listener; listens to source.state: a stream opened at main.js:"
    );
    assert!(log.contains(&resident), "{log}");
}
