//! B0 of the plugin process supervisor (doc/architecture/supervisor.md
//! §13): characterization tests of today's plugin lifecycle with the real
//! host, headless, so that batches B2a–B2d can show zero behaviour
//! difference. They pin what the other suites do not:
//!
//! | Behaviour | Where it is pinned |
//! | --- | --- |
//! | Built-ins first, first runs eight at a time, built-in first runs before third-party ones | `lazy_activation.rs` (`first_runs_start_eight_at_a_time`, `built_in_first_runs_go_before_other_plugins`); `StartQueue`/`FirstQueue` unit tests |
//! | Park, replay, wake by a command, wake while the parked process still stops, idle stop reason | `lazy_activation.rs` (`an_idle_plugin_is_parked_replayed_woken_by_its_command_and_parked_again`) |
//! | Resident at ready, mirrors park | `lazy_activation.rs` (`mirror_subscriptions_park_and_wake_a_listener_keeps_a_plugin_resident`) |
//! | Wake events, first announcement muted | `lazy_activation.rs` (`a_packages_listener_is_parked_and_woken_by_a_package_change`) |
//! | Stop reasons (reload, failed, disable, uninstall, upgrade, cancel, quit) | `shared_runner.rs` (`every_stop_tells_the_plugin_why`) |
//! | Runner restart, fail-open after 3 in 60 s, quarantine, hang | `shared_runner.rs` |
//! | Reload of a shared-runner member waits for its slow cleanup | `shared_runner.rs` (`a_reload_waits_for_a_slow_cleanup_to_finish`) |
//! | Install `enabled`, restore, start-up recovery | `package_install.rs` |
//! | Held triggers delivered in order at the ready | here: `held_commands_reach_a_woken_plugin_in_order_at_its_ready` |
//! | Own-process reload and upgrade wait for the old process; uninstall data removed after the reap; session values per stop reason; `busy`/`unsupported`/`invalid`/`missing` replies | here: `own_process_restarts_wait_for_the_old_process_and_session_values_follow_the_reason` |
//! | `packages.list` of a parked plugin; a park announces nothing | here: `a_parked_plugin_is_listed_running_idle_and_its_park_announces_nothing` |
//! | Known bug A18-1 (Book entry leaked on a failed spawn) | here: `b0_known_bug_a_failed_spawn_leaks_its_activation_place` |
//! | Known bug A18-2 (`Host.cleanup` only grows) | here: `b0_known_bug_host_cleanup_only_grows` |
//! | Known bug D13 (disable then enable runs two processes) | here: `b0_known_bug_disable_then_enable_starts_beside_the_old_process` |
//!
//! Built-ins from a `builtin.json` schema 3 under `VESTRA_BUILTIN_PLUGINS`,
//! the development runner, Node 26 on `PATH` (or `VESTRA_NODE`) and libmpv.
//! Ignored by default; `npm run ci:local` runs them (`--include-ignored`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

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

/// Package `id` with `capabilities` and `main` written to `directory`; its
/// manifest and digest.
fn package(
    directory: &Path,
    id: &str,
    version: &str,
    capabilities: &[&str],
    main: &str,
) -> (Value, String) {
    let manifest = json!({"id":id,"version":version,"apiVersion":"vestra/1",
        "entry":"main.js","capabilities":capabilities});
    let files = [
        ("manifest.json", manifest.to_string()),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main.to_owned()),
    ];
    std::fs::create_dir_all(directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    (manifest, digest(&files))
}

/// A built-in under `plugins`: its `builtin.json` entry.
fn builtin(plugins: &Path, id: &str, capabilities: &[&str], main: &str) -> Value {
    let (_, digest) = package(&plugins.join(id), id, "1.0.0", capabilities, main);
    json!({"id":id,"version":"1.0.0","directory":id,"digest":digest})
}

/// A player data root: `plugins/builtin.json` with `entries`, and a store
/// whose catalog installs `installed` (third-party plugins: own processes).
struct Root {
    dir: tempfile::TempDir,
    catalog: Value,
    entries: Vec<Value>,
}
impl Root {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("plugins")).unwrap();
        std::fs::create_dir_all(dir.path().join("store").join("packages")).unwrap();
        Self {
            dir,
            catalog: json!({"installed":{},"pending":{},"operations":{},"lastCommit":{}}),
            entries: Vec::new(),
        }
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn builtin(&mut self, id: &str, capabilities: &[&str], main: &str) {
        let entry = builtin(&self.path().join("plugins"), id, capabilities, main);
        self.entries.push(entry);
    }
    /// Installs third-party `id` as instance `n` (its data directory is
    /// `data/<guid>`).
    fn install(&mut self, n: usize, id: &str, main: &str) -> PathBuf {
        let store = self.path().join("store");
        let staging = store.join("packages").join(format!("staging-{id}"));
        let (manifest, digest) = package(&staging, id, "1.0.0", &[], main);
        let directory = store.join("packages").join(&digest);
        std::fs::rename(&staging, &directory).unwrap();
        let guid = format!("00000000-0000-4000-8000-{n:012x}");
        self.catalog["installed"][id] = json!({"revision":1,
            "current":{"digest":digest,"manifest":manifest,"directory":directory},
            "previous":null,
            "instance":{"guid":guid,"source":{"kind":"file"},"installedAt":1}});
        self.path().join("data").join(guid)
    }
    /// Runs the host headless for `seconds`; its stderr.
    fn run(&self, seconds: u32, env: &[(&str, &str)]) -> String {
        std::fs::write(
            self.path().join("plugins").join("builtin.json"),
            json!({"schema":3,"plugins":self.entries}).to_string(),
        )
        .unwrap();
        // Once: later runs keep the store's own records (the built-ins'
        // instances, which recordings are keyed by).
        let catalog = self.path().join("store").join("catalog.json");
        if !catalog.exists() {
            std::fs::write(catalog, self.catalog.to_string()).unwrap();
        }
        let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
        command
            .args(["--headless", "--silent", "--seconds", &seconds.to_string()])
            .env("VESTRA_BUILTIN_PLUGINS", self.path().join("plugins"))
            .env("VESTRA_PACKAGE_STORE", self.path().join("store"))
            .env("VESTRA_PLUGIN_DATA", self.path().join("data"));
        for key in [
            "VESTRA_SHARED",
            "VESTRA_LAZY",
            "VESTRA_IDLE_MS",
            "VESTRA_TEST_EXECUTE",
            "VESTRA_TEST_PARK",
            "VESTRA_TEST_WAKE",
            "VESTRA_TEST_REGISTRY",
            "VESTRA_INSTANCE_DIR",
        ] {
            command.env_remove(key);
        }
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command.output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{stderr}");
        stderr
    }
}

/// The position of `text` in `log`, or a panic with the log.
fn at(log: &str, text: &str) -> usize {
    log.find(text)
        .unwrap_or_else(|| panic!("no {text:?} in\n{log}"))
}

const TWO: &str = "vestra.fixture-two";

/// A woken plugin receives every trigger held for it, in the order they
/// came, at its `runtime.ready` (`lazy_ready`): two commands executed in
/// the same tick (the first wakes the parked owner, the second goes to the
/// waking one) both run, first then second, after the start.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn held_commands_reach_a_woken_plugin_in_order_at_its_ready() {
    let mut root = Root::new();
    root.builtin(
        TWO,
        &["commands"],
        &format!(
            "export default function activate(plugin) {{\n\
             console.error(`{TWO} start`);\n\
             for (const id of [\"first\", \"second\"])\n\
             plugin.commands.add({{ id, path: [\"Tools\", id], run: () => console.error(`{TWO} ran ${{id}}`) }});\n}}\n"
        ),
    );
    let lazy = [
        ("VESTRA_SHARED", "1"),
        ("VESTRA_LAZY", "1"),
        ("VESTRA_IDLE_MS", "500"),
    ];
    // A first run: lazy at its ready, parked at once, recorded.
    let first = root.run(4, &lazy);
    at(&first, &format!("lazy: {TWO} parked (2 recorded calls)"));

    let mut woken = lazy.to_vec();
    woken.push((
        "VESTRA_TEST_EXECUTE",
        "vestra.fixture-two/first@2500,vestra.fixture-two/second@2500",
    ));
    let second = root.run(6, &woken);
    let replayed = at(&second, &format!("lazy: {TWO} replayed (2 calls)"));
    let woke = at(&second, &format!("lazy: {TWO} woken by commands.execute"));
    let start = at(&second, &format!("] {TWO} start"));
    assert!(
        second.contains(" ms, 2 trigger(s)"),
        "both commands held for the ready: {second}"
    );
    let ran_first = at(&second, &format!("{TWO} ran first"));
    let ran_second = at(&second, &format!("{TWO} ran second"));
    assert!(
        replayed < woke && woke < start && start < ran_first && ran_first < ran_second,
        "{second}"
    );
    assert_eq!(
        second.matches(&format!("] {TWO} start")).count(),
        1,
        "{second}"
    );
    assert_eq!(
        second.matches(&format!("{TWO} ran ")).count(),
        2,
        "{second}"
    );
}

/// A third-party plugin that logs each start (`<id> <version> start found
/// [<marker>] mark <session value>`) and whose cleanup writes its data
/// directory's marker, `started` then, 100 ms later, ` finished`. The data
/// directory is created again by the cleanup, so a removal before the
/// process is gone would leave it behind.
fn tracked(id: &str, version: &str) -> String {
    format!(
        "import {{ readFileSync, writeFileSync, appendFileSync, mkdirSync }} from \"node:fs\";\n\
         export default async function activate(plugin) {{\n\
         const dir = plugin.dataDirectory;\n\
         const marker = `${{dir}}/cleanup.txt`;\n\
         let before = \"\"; try {{ before = readFileSync(marker, \"utf8\"); }} catch {{}}\n\
         const mark = (await plugin.storage.session.get(\"mark\")) ?? \"none\";\n\
         await plugin.storage.session.set(\"mark\", \"set\");\n\
         console.error(`{id} {version} start found [${{before}}] mark ${{mark}}`);\n\
         plugin.onDispose(async (info) => {{\n\
         mkdirSync(dir, {{ recursive: true }}); writeFileSync(marker, \"started\");\n\
         await new Promise((done) => setTimeout(done, 100));\n\
         mkdirSync(dir, {{ recursive: true }}); appendFileSync(marker, \" finished\");\n\
         console.error(`{id} {version} stopped for ${{info?.reason}}`);\n\
         }});\n}}\n"
    )
}

const RELOAD: &str = "aa.fixture-reload";
const UPGRADE: &str = "aa.fixture-upgrade";
const TOGGLE: &str = "aa.fixture-toggle";
const GONE: &str = "aa.fixture-gone";
const MANAGER: &str = "vestra.fixture-manager";

/// Own processes (third-party plugins): a reload and an upgrade start the
/// new instance only once the old one's cleanup is over (`reloads` +
/// `reload_tick`, `Operation.waiting` + `advance_package`); an uninstall
/// with `removeData` deletes the data directory only once the process is
/// gone (`data_removals` at `Reaped`); session values survive a reload and
/// an upgrade and go with a user disable (`session_storage.rs`). The
/// replies a lifecycle request gets while another runs or when it is not
/// allowed: `busy`, `unsupported`, `invalid`, `missing`.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn own_process_restarts_wait_for_the_old_process_and_session_values_follow_the_reason() {
    let mut root = Root::new();
    root.install(1, RELOAD, &tracked(RELOAD, "1.0.0"));
    root.install(2, UPGRADE, &tracked(UPGRADE, "1.0.0"));
    root.install(3, TOGGLE, &tracked(TOGGLE, "1.0.0"));
    let gone_data = root.install(4, GONE, &tracked(GONE, "1.0.0"));
    let candidate = root.path().join("candidates").join(UPGRADE);
    package(
        &candidate,
        UPGRADE,
        "2.0.0",
        &[],
        &tracked(UPGRADE, "2.0.0"),
    );
    let candidate = serde_json::to_string(&candidate.to_string_lossy()).unwrap();
    root.builtin(
        "vestra.fixture-calm",
        &[],
        "export default function activate() {}\n",
    );
    root.builtin(
        MANAGER,
        &["plugins.manage"],
        &format!(
            "const log = (text) => console.error(`{MANAGER} ${{text}}`);\n\
             const code = (promise) => promise.then(() => \"ok\", (e) => e.code);\n\
             const sleep = (ms) => new Promise((done) => setTimeout(done, ms));\n\
             export default function activate(plugin) {{\n\
             const p = plugin.packages;\n\
             setTimeout(async () => {{\n\
             const first = code(p.reload(\"{RELOAD}\"));\n\
             const again = code(p.reload(\"{RELOAD}\"));\n\
             const toggle = code(p.setEnabled(\"{RELOAD}\", false));\n\
             log(`reload ${{await first}} again ${{await again}} disable ${{await toggle}}`);\n\
             log(`builtin ${{await code(p.setEnabled(\"vestra.fixture-calm\", false))}}`);\n\
             log(`self ${{await code(p.setEnabled(\"{MANAGER}\", false))}}`);\n\
             log(`missing ${{await code(p.reload(\"aa.fixture-nothing\"))}}`);\n\
             log(`off ${{await code(p.setEnabled(\"{TOGGLE}\", false))}}`);\n\
             log(`reload disabled ${{await code(p.reload(\"{TOGGLE}\"))}}`);\n\
             await sleep(1500);\n\
             log(`on ${{await code(p.setEnabled(\"{TOGGLE}\", true))}}`);\n\
             try {{\n\
             const update = await (await p.validate({candidate})).install();\n\
             log(`update ${{await update.done}}`);\n\
             const removal = await p.uninstall(\"{GONE}\", {{ removeData: true }});\n\
             log(`uninstall ${{await removal.done}}`);\n\
             }} catch (e) {{ log(`failed ${{e.code}} ${{e.message}}`); }}\n\
             log(\"done\");\n\
             }}, 1500);\n}}\n"
        ),
    );
    let log = root.run(10, &[("VESTRA_SHARED", "0"), ("VESTRA_LAZY", "0")]);
    at(&log, &format!("{MANAGER} done"));
    for expected in [
        format!("{MANAGER} reload ok again busy disable busy"),
        format!("{MANAGER} builtin unsupported"),
        format!("{MANAGER} self invalid"),
        format!("{MANAGER} missing missing"),
        format!("{MANAGER} off ok"),
        format!("{MANAGER} reload disabled invalid"),
        format!("{MANAGER} on ok"),
        format!("{MANAGER} update committed"),
        format!("{MANAGER} uninstall committed"),
        // The new instance found the old one's cleanup finished; a reload
        // and an upgrade keep the session values.
        format!("{RELOAD} 1.0.0 stopped for reload"),
        format!("{RELOAD} 1.0.0 start found [started finished] mark set"),
        format!("{UPGRADE} 1.0.0 stopped for upgrade"),
        format!("{UPGRADE} 2.0.0 start found [started finished] mark set"),
        // A user disable clears them.
        format!("{TOGGLE} 1.0.0 stopped for disable"),
        format!("{TOGGLE} 1.0.0 start found [started finished] mark none"),
        format!("{GONE} 1.0.0 stopped for uninstall"),
    ] {
        at(&log, &expected);
    }
    // Each plugin starts once per instance: the reload once more.
    assert_eq!(
        log.matches(&format!("{RELOAD} 1.0.0 start")).count(),
        2,
        "{log}"
    );
    assert_eq!(
        log.matches(&format!("{GONE} 1.0.0 start")).count(),
        1,
        "{log}"
    );
    // The cleanup wrote the data directory again before its process went:
    // it was removed after that.
    assert!(log.contains(&format!("{GONE} 1.0.0 stopped for uninstall")));
    assert!(
        !gone_data.exists(),
        "the uninstalled plugin's data is left: {}\n{log}",
        gone_data.display()
    );
}

const IDLE: &str = "vestra.fixture-idle";
const WATCH: &str = "vestra.fixture-watch";

/// `packages.list` of a parked plugin: `state: "running"`, `process:
/// "idle"`; its park alone changes only `process`, which `packages.changed`
/// leaves out of its comparison, so a listener hears its start once and
/// its park never.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_parked_plugin_is_listed_running_idle_and_its_park_announces_nothing() {
    let mut root = Root::new();
    root.builtin(
        IDLE,
        &["commands"],
        "export default function activate(plugin) {\n\
         plugin.commands.add({ id: \"hello\", path: [\"Tools\", \"Hello\"], run: () => {} });\n}\n",
    );
    root.builtin(
        WATCH,
        &["events", "playback.read", "plugins.manage"],
        &format!(
            "const shown = (list) => {{ const p = list.find((p) => p.id === \"{IDLE}\"); return `${{p?.state}} ${{p?.process}}`; }};\n\
             export default function activate(plugin) {{\n\
             plugin.player.onChange(() => {{}});\n\
             plugin.packages.onChange((list) => console.error(`{WATCH} changed ${{shown(list)}}`));\n\
             setTimeout(async () => console.error(`{WATCH} listed ${{shown(await plugin.packages.list())}}`), 3500);\n}}\n"
        ),
    );
    let log = root.run(
        5,
        &[
            ("VESTRA_SHARED", "1"),
            ("VESTRA_LAZY", "1"),
            ("VESTRA_IDLE_MS", "500"),
        ],
    );
    let parked = at(&log, &format!("lazy: {IDLE} parked"));
    let listed = at(&log, &format!("{WATCH} listed running idle"));
    assert!(parked < listed, "{log}");
    at(&log, &format!("lazy: {WATCH} classified resident"));
    // Once `running` (with its process, or already idle if the first
    // announcement came after the park), never again for the park.
    let running = log.matches(&format!("{WATCH} changed running")).count();
    assert!(running <= 1, "the park was announced: {log}");
}

/// B0 known bug: expected to change in B2a (supervisor.md §0 D3, A18 (1)).
/// `load_package` takes an activation place (`activations.begin`) before
/// it spawns, and an early return (`Plugin::load_manifest(..)?`) never gives
/// it back: after 64 failed spawns the Book is full and every later start
/// fails with "activation tracking limit reached" instead of its own error.
/// Node is pointed at a missing file, so no plugin process runs at all.
#[test]
#[ignore = "needs libmpv: run with --include-ignored (ci:local)"]
fn b0_known_bug_a_failed_spawn_leaks_its_activation_place() {
    let mut root = Root::new();
    // Installed (third-party) plugins: `builtin.json` lists at most 64.
    for n in 0..65 {
        root.install(
            n + 1,
            &format!("aa.fixture-{n:03}"),
            "export default function activate() {}\n",
        );
    }
    let node = root.path().join("no-node").join("node.exe");
    let node = node.to_string_lossy();
    let log = root.run(
        3,
        &[
            ("VESTRA_SHARED", "0"),
            ("VESTRA_LAZY", "0"),
            ("VESTRA_NODE", node.as_ref()),
        ],
    );
    let failures = log
        .lines()
        .filter(|line| line.contains("failed to start: "))
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 65, "{log}");
    // They start in id order: the first 64 fail to spawn Node...
    for (n, line) in failures.iter().take(64).enumerate() {
        assert!(
            line.contains(&format!("aa.fixture-{n:03} failed to start: start Node")),
            "{line}\n{log}"
        );
    }
    // ...and each leaks its place: the 65th finds the Book full.
    // B0 known bug: expected to change in B2a (it then fails to spawn Node
    // like the others).
    assert!(
        failures[64].contains("aa.fixture-064 failed to start: activation tracking limit reached"),
        "{log}"
    );
}

/// B0 known bug: expected to change in B2b (supervisor.md §4.7, A18 (2)).
/// `Host.cleanup` (whether a stopped owner's process is reaped and the
/// playback worker cleaned up after it) gets an entry at every stop and
/// none leaves it in a run: no code removes one. No headless output shows
/// its size, so the source is read: every write to it is an `insert` or a
/// `get_mut`. B2b replaces it by the unit's stopping flags.
#[test]
fn b0_known_bug_host_cleanup_only_grows() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut writes = Vec::new();
    let mut stack = vec![source];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                // Without whitespace: a call split over lines counts too.
                let text = std::fs::read_to_string(&path)
                    .unwrap()
                    .split_whitespace()
                    .collect::<String>();
                for (_, rest) in text
                    .match_indices("self.cleanup.")
                    .map(|(i, m)| text.split_at(i + m.len()))
                {
                    let call = rest.split('(').next().unwrap_or_default().to_owned();
                    writes.push((path.clone(), call));
                }
            }
        }
    }
    let calls = writes
        .iter()
        .map(|(_, call)| call.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    // B0 known bug: expected to change in B2b (`remove`, or the field gone).
    assert_eq!(
        calls,
        ["get", "get_mut", "insert"].into_iter().collect(),
        "{writes:?}"
    );
}

const TWICE: &str = "aa.fixture-twice";

/// B0 known bug: expected to change in B2b (supervisor.md §0 D13).
/// `packages.setEnabled(false)` then `setEnabled(true)` on a running
/// plugin: the stop takes it out of `plugins` at once, `parked_process`
/// looks only at parked processes, so the enable starts a new process on
/// the same data directory while the old one is still in its 300 ms grace
/// (here it never finishes its cleanup, so it is killed at the end of it).
/// Each instance notes its pid in its data directory when its module loads
/// and logs the other instances' processes still alive then. A start that
/// waited for the reap would log `overlaps []` every time; today at least
/// one of three toggles starts beside the old process (2026-11-02, Windows
/// laptop: all three did, each new module loaded while the old pid lived).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn b0_known_bug_disable_then_enable_starts_beside_the_old_process() {
    let mut root = Root::new();
    root.install(
        1,
        TWICE,
        &format!(
            "import {{ readdirSync, writeFileSync }} from \"node:fs\";\n\
             const dir = process.env.VESTRA_PLUGIN_DATA_DIR;\n\
             const alive = (pid) => {{ try {{ process.kill(pid, 0); return true; }} catch {{ return false; }} }};\n\
             const others = readdirSync(dir).filter((f) => f.startsWith(\"alive-\"))\n\
             .map((f) => Number(f.slice(6))).filter((pid) => pid !== process.pid && alive(pid));\n\
             writeFileSync(`${{dir}}/alive-${{process.pid}}`, \"\");\n\
             console.error(`{TWICE} pid ${{process.pid}} overlaps [${{others}}]`);\n\
             export default function activate(plugin) {{\n\
             plugin.onDispose(() => new Promise(() => {{}}));\n}}\n"
        ),
    );
    root.builtin(
        MANAGER,
        &["plugins.manage"],
        &format!(
            "const sleep = (ms) => new Promise((done) => setTimeout(done, ms));\n\
             export default function activate(plugin) {{\n\
             setTimeout(async () => {{\n\
             for (let i = 0; i < 3; i++) {{\n\
             const off = plugin.packages.setEnabled(\"{TWICE}\", false);\n\
             const on = plugin.packages.setEnabled(\"{TWICE}\", true);\n\
             const codes = await Promise.all([off, on].map((p) => p.then(() => \"ok\", (e) => e.code)));\n\
             console.error(`{MANAGER} toggled ${{codes}}`);\n\
             await sleep(1500);\n\
             }}\n\
             console.error(`{MANAGER} done`);\n\
             }}, 1500);\n}}\n"
        ),
    );
    let log = root.run(9, &[("VESTRA_SHARED", "0"), ("VESTRA_LAZY", "0")]);
    at(&log, &format!("{MANAGER} done"));
    assert_eq!(
        log.matches(&format!("{MANAGER} toggled ok,ok")).count(),
        3,
        "{log}"
    );
    let starts = log
        .lines()
        .filter(|line| line.contains(&format!("{TWICE} pid ")))
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 4, "the first start and three enables: {log}");
    let overlapping = starts
        .iter()
        .filter(|line| !line.ends_with("overlaps []"))
        .count();
    // B0 known bug: expected to change in B2b (then 0: an enable waits for
    // the old process).
    assert!(
        overlapping >= 1,
        "no enable started beside the old process: {log}"
    );
}
