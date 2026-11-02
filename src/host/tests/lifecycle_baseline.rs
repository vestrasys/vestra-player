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
//! | A cleanup that never ends: reload (shared runner) and upgrade (own process) wait only for the stop's grace, the 20 s texts never appear; `busy` while an operation runs | here: `an_endless_cleanup_holds_a_reload_or_an_upgrade_only_for_its_grace` |
//! | Standby with a process still stopping: wake waits, enable goes through `packages.reloads` (`starting`, `busy`), uninstall removes the data after the reap (or at once once reaped) | here: `standby_holds_wake_enable_and_uninstall_until_the_parked_process_is_gone` |
//! | Quit stops own processes one after another (300 ms each) | here: `quit_stops_own_processes_one_after_another` |
//! | `packages.list` `starting`, `failed` (with its reason), `disabled` | here: `packages_list_shows_starting_failed_and_disabled` |
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
        self.install_with(n, id, &[], main)
    }
    /// `install` with these manifest capabilities.
    fn install_with(&mut self, n: usize, id: &str, capabilities: &[&str], main: &str) -> PathBuf {
        let store = self.path().join("store");
        let staging = store.join("packages").join(format!("staging-{id}"));
        let (manifest, digest) = package(&staging, id, "1.0.0", capabilities, main);
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
        self.run_timed(seconds, env).0
    }
    /// `run`, and how long the host process took from spawn to exit.
    fn run_timed(&self, seconds: u32, env: &[(&str, &str)]) -> (String, std::time::Duration) {
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
        let began = std::time::Instant::now();
        let output = command.output().unwrap();
        let took = began.elapsed();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{stderr}");
        (stderr, took)
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

/// Module code (it runs when the plugin's module loads, before its
/// activation): `others` is the pids of this plugin's earlier instances
/// whose process is still alive, each instance noting its own pid in its
/// data directory (`alive-<pid>`). `overlaps [${others}]` is `[]` exactly
/// when no earlier process of the plugin still runs (own processes only:
/// shared-runner members share one pid and get no `VESTRA_PLUGIN_DATA_DIR`:
/// always `[]` there).
const ALIVE: &str = "import { readdirSync as aliveList, writeFileSync as aliveNote } from \"node:fs\";\n\
const aliveDir = process.env.VESTRA_PLUGIN_DATA_DIR;\n\
const alive = (pid) => { try { process.kill(pid, 0); return true; } catch { return false; } };\n\
const others = !aliveDir ? [] : aliveList(aliveDir).filter((f) => f.startsWith(\"alive-\"))\n\
.map((f) => Number(f.slice(6))).filter((pid) => pid !== process.pid && alive(pid));\n\
if (aliveDir) aliveNote(`${aliveDir}/alive-${process.pid}`, \"\");\n";

/// How a `fixture`'s cleanup (`onDispose`) behaves.
#[derive(Clone, Copy)]
enum Cleanup {
    /// Writes its marker, `started` then, 100 ms later, ` finished`, and
    /// logs `<id> <version> stopped for <reason>`.
    Quick,
    /// Logs `<id> <version> stopping for <reason>`, writes `started`, then
    /// writes its data directory every 50 ms (creating it again) and never
    /// returns: only the end of its process ends it.
    Endless,
}

/// A plugin that logs each start, `<id> <version> start found [<marker>]
/// mark <session value> overlaps [<pids>]` (`ALIVE`), after `delay_ms` of
/// its activation (which first runs `extra`), and whose cleanup is
/// `cleanup`. Both cleanups write the
/// data directory again, so a removal before the process is gone leaves
/// it behind.
fn fixture(id: &str, version: &str, cleanup: Cleanup, delay_ms: u32, extra: &str) -> String {
    let dispose = match cleanup {
        Cleanup::Quick => format!(
            "mkdirSync(dir, {{ recursive: true }}); writeFileSync(marker, \"started\");\n\
             await new Promise((done) => setTimeout(done, 100));\n\
             mkdirSync(dir, {{ recursive: true }}); appendFileSync(marker, \" finished\");\n\
             console.error(`{id} {version} stopped for ${{info?.reason}}`);\n"
        ),
        Cleanup::Endless => format!(
            "console.error(`{id} {version} stopping for ${{info?.reason}}`);\n\
             mkdirSync(dir, {{ recursive: true }}); writeFileSync(marker, \"started\");\n\
             setInterval(() => {{ try {{ mkdirSync(dir, {{ recursive: true }});\n\
             writeFileSync(`${{dir}}/late.txt`, String(Date.now())); }} catch {{}} }}, 50);\n\
             await new Promise(() => {{}});\n"
        ),
    };
    format!(
        "{ALIVE}import {{ readFileSync, writeFileSync, appendFileSync, mkdirSync }} from \"node:fs\";\n\
         export default async function activate(plugin) {{\n\
         const dir = plugin.dataDirectory;\n\
         const marker = `${{dir}}/cleanup.txt`;\n\
         plugin.onDispose(async (info) => {{\n{dispose}}});\n\
         {extra}\n\
         let before = \"\"; try {{ before = readFileSync(marker, \"utf8\"); }} catch {{}}\n\
         const mark = (await plugin.storage.session.get(\"mark\")) ?? \"none\";\n\
         await plugin.storage.session.set(\"mark\", \"set\");\n\
         await new Promise((done) => setTimeout(done, {delay_ms}));\n\
         console.error(`{id} {version} start found [${{before}}] mark ${{mark}} overlaps [${{others}}]`);\n}}\n"
    )
}

/// `fixture` with a quick cleanup and no delay.
fn tracked(id: &str, version: &str) -> String {
    fixture(id, version, Cleanup::Quick, 0, "")
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
        // The new instance loaded once the old process was gone (`overlaps
        // []`, which a cleanup marker alone could not show: the 100 ms
        // cleanup ends within the 300 ms grace either way); a reload and an
        // upgrade keep the session values.
        format!("{RELOAD} 1.0.0 stopped for reload"),
        format!("{RELOAD} 1.0.0 start found [started finished] mark set overlaps []"),
        format!("{UPGRADE} 1.0.0 stopped for upgrade"),
        format!("{UPGRADE} 2.0.0 start found [started finished] mark set overlaps []"),
        // A user disable clears them.
        format!("{TOGGLE} 1.0.0 stopped for disable"),
        format!("{TOGGLE} 1.0.0 start found [started finished] mark none overlaps []"),
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
/// its size, so the source is read: no use of any receiver's `.cleanup`
/// (the manager's `packages.cleanup`, a library thread, aside) removes
/// entries or replaces the map. Reads (`get`, `contains_key`, `len`,
/// `iter`) and `insert` are not counted. B2b replaces the map by the unit's
/// stopping flags.
#[test]
fn b0_known_bug_host_cleanup_only_grows() {
    let removals = cleanup_removals(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
    // B0 known bug: expected to change in B2b (a removal appears, or the
    // field goes and this test with it).
    assert!(removals.is_empty(), "{removals:?}");
    // The scanner itself finds each kind of removal, and skips reads,
    // inserts and the manager's own `cleanup`.
    let scratch = tempfile::tempdir().unwrap();
    std::fs::write(
        scratch.path().join("a.rs"),
        "self.cleanup.get(&o); self.cleanup.insert(o, x); host.cleanup.len();\n\
         self.packages.cleanup = None; self.packages\n.cleanup.take();\n\
         self.cleanup_done(o); self.cleanup_tick();\n\
         self.cleanup\n    .remove(&o); host.cleanup.retain(|_, _| true); self.cleanup.clear();\n\
         self.cleanup.pop_first(); x.cleanup.pop_last(); self.cleanup.split_off(&o);\n\
         self.cleanup.extract_if(|_, _| true); self.cleanup = BTreeMap::new();\n\
         std::mem::take(&mut self.cleanup); mem::replace(&mut host.cleanup, m);\n\
         self.cleanup == other;\n",
    )
    .unwrap();
    let found = cleanup_removals(scratch.path())
        .into_iter()
        .map(|(_, what)| what)
        .collect::<Vec<_>>();
    assert_eq!(
        found,
        [
            "remove",
            "retain",
            "clear",
            "pop_first",
            "pop_last",
            "split_off",
            "extract_if",
            "=",
            "take",
            "replace"
        ],
        "{found:?}"
    );
}

/// Every use under `root` (`.rs` files) of a `.cleanup` field, other than
/// `packages.cleanup`, that removes entries or replaces the whole map:
/// (file, the method, `=`, or the `mem::` function).
fn cleanup_removals(root: &Path) -> Vec<(PathBuf, String)> {
    const REMOVING: [&str; 7] = [
        "remove",
        "retain",
        "clear",
        "pop_first",
        "pop_last",
        "split_off",
        "extract_if",
    ];
    let mut found = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(directory) = stack.pop() {
        let mut entries = std::fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            // Without whitespace: a use split over lines counts too.
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .split_whitespace()
                .collect::<String>();
            for (at, _) in text.match_indices(".cleanup") {
                let before = &text[..at];
                let after = &text[at + ".cleanup".len()..];
                // `cleanup_done`, `cleanup_tick`, `cleanup_queue`: other names.
                if after
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
                    || before.ends_with("packages")
                {
                    continue;
                }
                let what = if let Some(call) = after.strip_prefix('.') {
                    let method = call.split('(').next().unwrap_or_default();
                    REMOVING.contains(&method).then(|| method.to_owned())
                } else if after.starts_with('=') && !after.starts_with("==") {
                    Some("=".to_owned())
                } else if after.starts_with(')') {
                    // `mem::take(&mut x.cleanup)`, `mem::replace(&mut x.cleanup, ..)`.
                    let receiver = before.rfind("(&mut").map(|i| &before[..i]);
                    receiver.and_then(|head| {
                        ["take", "replace"]
                            .into_iter()
                            .find(|f| head.ends_with(&format!("mem::{f}")))
                            .map(str::to_owned)
                    })
                } else if after.starts_with(',') {
                    let receiver = before.rfind("(&mut").map(|i| &before[..i]);
                    receiver
                        .filter(|head| head.ends_with("mem::replace"))
                        .map(|_| "replace".to_owned())
                } else {
                    None
                };
                if let Some(what) = what {
                    found.push((path.clone(), what));
                }
            }
        }
    }
    found
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
            "{ALIVE}console.error(`{TWICE} pid ${{process.pid}} overlaps [${{others}}]`);\n\
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

/// The position of the `n`th (from 1) `text` in `log`, or a panic.
fn nth(log: &str, text: &str, n: usize) -> usize {
    log.match_indices(text)
        .nth(n - 1)
        .map(|(at, _)| at)
        .unwrap_or_else(|| panic!("no {n}th {text:?} in\n{log}"))
}

/// The JavaScript string literal of `path`.
fn js_path(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

const ENDLESS: &str = "vestra.fixture-endless";
const ENDLESS_UP: &str = "aa.fixture-endless-up";
const BUSY_TEXT: &str = "busy: the package is being installed or reloaded";

/// The 20 s waits (`RELOAD_WAIT_MS` in `reload_tick`, the install wait in
/// `advance_package`) never run out today: every stop ends its process
/// within the stop's own grace, and the restart goes on then. A cleanup
/// that never ends holds a reload of a shared-runner member until the
/// runner abandons it (its 5 s `STOP_TIMEOUT_MS`, inside the host's 6 s
/// `MEMBER_GONE`), and an upgrade of an own process for its 300 ms grace;
/// neither failure text appears. While the upgrade's operation runs, the
/// plugin's `setEnabled`, `reload` and `uninstall` are `busy` (the
/// operation half of `package_busy`).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn an_endless_cleanup_holds_a_reload_or_an_upgrade_only_for_its_grace() {
    let mut root = Root::new();
    root.builtin(
        ENDLESS,
        &[],
        &fixture(ENDLESS, "1.0.0", Cleanup::Endless, 0, ""),
    );
    root.install(
        1,
        ENDLESS_UP,
        &fixture(ENDLESS_UP, "1.0.0", Cleanup::Endless, 0, ""),
    );
    let candidate = root.path().join("candidates").join(ENDLESS_UP);
    // Activates for 3 s: the operation is in `activating` meanwhile.
    package(
        &candidate,
        ENDLESS_UP,
        "2.0.0",
        &[],
        &fixture(ENDLESS_UP, "2.0.0", Cleanup::Quick, 3000, ""),
    );
    let candidate = js_path(&candidate);
    root.builtin(
        MANAGER,
        &["plugins.manage"],
        &format!(
            "const log = (text) => console.error(`{MANAGER} ${{text}}`);\n\
             const code = (promise) => promise.then(() => \"ok\", (e) => `${{e.code}}: ${{e.message}}`);\n\
             export default function activate(plugin) {{\n\
             const p = plugin.packages;\n\
             setTimeout(async () => {{\n\
             log(`reload ${{await code(p.reload(\"{ENDLESS}\"))}}`);\n\
             try {{\n\
             const update = await (await p.validate({candidate})).install();\n\
             let asked = false;\n\
             update.onChange(async (status) => {{\n\
             if (status !== \"activating\" || asked) return;\n\
             asked = true;\n\
             log(`activating: disable ${{await code(p.setEnabled(\"{ENDLESS_UP}\", false))}}`);\n\
             log(`activating: reload ${{await code(p.reload(\"{ENDLESS_UP}\"))}}`);\n\
             log(`activating: uninstall ${{await code(p.uninstall(\"{ENDLESS_UP}\"))}}`);\n\
             }});\n\
             log(`update ${{await update.done}}`);\n\
             }} catch (e) {{ log(`failed ${{e.code}} ${{e.message}}`); }}\n\
             log(\"done\");\n\
             }}, 1500);\n}}\n"
        ),
    );
    let log = root.run(13, &[("VESTRA_SHARED", "1"), ("VESTRA_LAZY", "0")]);
    at(&log, &format!("{MANAGER} done"));
    at(&log, &format!("{MANAGER} reload ok"));
    // The member: still cleaning up after the host's 300 ms, abandoned by
    // the runner, then started again.
    let stopping = at(&log, &format!("{ENDLESS} 1.0.0 stopping for reload"));
    let waited = at(
        &log,
        "still cleaning up after 300 ms; waiting for the runner",
    );
    let abandoned = at(&log, "cleanup was abandoned after the runner's deadline");
    let again = nth(&log, &format!("{ENDLESS} 1.0.0 start found"), 2);
    assert!(
        stopping < waited && waited < abandoned && abandoned < again,
        "{log}"
    );
    // The own process: killed after its grace; the new version loads once
    // it is gone and keeps the session values.
    at(&log, &format!("{ENDLESS_UP} 1.0.0 stopping for upgrade"));
    at(
        &log,
        &format!("{ENDLESS_UP} 2.0.0 start found [started] mark set overlaps []"),
    );
    for request in ["disable", "reload", "uninstall"] {
        at(
            &log,
            &format!("{MANAGER} activating: {request} {BUSY_TEXT}"),
        );
    }
    at(&log, &format!("{MANAGER} update committed"));
    for never in [
        "reload: the previous process did not stop within 20 s",
        "cleanup acknowledgment timed out",
    ] {
        assert!(!log.contains(never), "{never}: {log}");
    }
}

const WAKE: &str = "aa.fixture-wake";
const ENABLE: &str = "aa.fixture-enable";
const FRESH: &str = "aa.fixture-gone-fresh";
const REAPED: &str = "aa.fixture-gone-reaped";

/// Standby (lazy activation) with own processes whose cleanup never ends:
/// a park waits `PARK_WAIT` (2 s) for its process, and whatever comes
/// meanwhile waits for that process to go, never beside it: a wake by a
/// command (`Parked.waiting`, `lazy_tick`; the 20 s `STOP_WAIT_MS` never
/// runs out), a disable and enable (`set_enabled(true)` 鈫?`packages.reloads`:
/// listed `starting`, a second enable `busy`), an uninstall that removes the
/// data (`data_removals` at the parked process's `Reaped`). An uninstall
/// once the parked process is gone removes the data at once.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn standby_holds_wake_enable_and_uninstall_until_the_parked_process_is_gone() {
    let mut root = Root::new();
    root.install_with(
        1,
        WAKE,
        &["commands"],
        &fixture(
            WAKE,
            "1.0.0",
            Cleanup::Endless,
            0,
            &format!(
                "plugin.commands.add({{ id: \"hello\", path: [\"Tools\", \"Hello\"], run: () => console.error(`{WAKE} hello`) }});"
            ),
        ),
    );
    root.install(
        2,
        ENABLE,
        &fixture(ENABLE, "1.0.0", Cleanup::Endless, 0, ""),
    );
    let fresh = root.install(3, FRESH, &fixture(FRESH, "1.0.0", Cleanup::Endless, 0, ""));
    let reaped = root.install(
        4,
        REAPED,
        &fixture(REAPED, "1.0.0", Cleanup::Endless, 0, ""),
    );
    root.builtin(
        MANAGER,
        &["events", "playback.read", "plugins.manage"],
        &format!(
            "const log = (text) => console.error(`{MANAGER} ${{text}}`);\n\
             const code = (promise) => promise.then(() => \"ok\", (e) => e.code);\n\
             const sleep = (ms) => new Promise((done) => setTimeout(done, ms));\n\
             export default function activate(plugin) {{\n\
             plugin.player.onChange(() => {{}});\n\
             const p = plugin.packages;\n\
             const state = async (id) => {{ const e = (await p.list()).find((x) => x.id === id); return `${{e?.state}} ${{e?.process}}`; }};\n\
             const idle = async (id) => {{ while (!(await state(id)).endsWith(\" idle\")) await sleep(20); }};\n\
             (async () => {{\n\
             await idle(\"{ENABLE}\");\n\
             const off = code(p.setEnabled(\"{ENABLE}\", false));\n\
             const on = code(p.setEnabled(\"{ENABLE}\", true));\n\
             log(`enable off ${{await off}} on ${{await on}} then ${{await state(\"{ENABLE}\")}} again ${{await code(p.setEnabled(\"{ENABLE}\", true))}}`);\n\
             }})();\n\
             (async () => {{\n\
             await idle(\"{FRESH}\");\n\
             log(`fresh ${{await (await p.uninstall(\"{FRESH}\", {{ removeData: true }})).done}}`);\n\
             }})();\n\
             (async () => {{\n\
             await idle(\"{REAPED}\");\n\
             await sleep(3500);\n\
             log(`reaped ${{await (await p.uninstall(\"{REAPED}\", {{ removeData: true }})).done}}`);\n\
             }})();\n}}\n"
        ),
    );
    let log = root.run(
        11,
        &[
            ("VESTRA_SHARED", "0"),
            ("VESTRA_LAZY", "1"),
            ("VESTRA_IDLE_MS", "500"),
            ("VESTRA_TEST_EXECUTE", "aa.fixture-wake/hello@park+100"),
        ],
    );
    // The wake waited for the parked process, then started beside nothing.
    let parked = at(&log, &format!("lazy: {WAKE} parked"));
    let waits = at(&log, &format!("lazy: {WAKE} waits for its stopped process"));
    let woken = at(&log, &format!("lazy: {WAKE} woken by commands.execute"));
    let second = nth(&log, &format!("{WAKE} 1.0.0 start found"), 2);
    assert!(parked < waits && waits < woken && woken < second, "{log}");
    assert!(
        log[second..]
            .lines()
            .next()
            .is_some_and(|line| line.ends_with("overlaps []")),
        "{log}"
    );
    let hello = at(&log, &format!("{WAKE} hello"));
    assert!(second < hello, "{log}");
    // Enabled again right after its disable in standby: `starting` until
    // the parked process is gone, `busy` meanwhile; then a fresh start
    // (the disable cleared its session values) beside nothing.
    at(&log, &format!("lazy: {ENABLE} parked"));
    at(
        &log,
        &format!("{MANAGER} enable off ok on ok then starting"),
    );
    assert!(
        log.contains(" again busy"),
        "a second enable while waiting: {log}"
    );
    at(
        &log,
        &format!("{ENABLE} 1.0.0 start found [started] mark none overlaps []"),
    );
    // Uninstalled while the parked process still wrote its data: removed
    // after it went. Uninstalled once it was gone: removed at once.
    at(&log, &format!("{MANAGER} fresh committed"));
    at(&log, &format!("{MANAGER} reaped committed"));
    for (data, what) in [(&fresh, "just parked"), (&reaped, "parked and gone")] {
        assert!(
            !data.exists(),
            "{what}: its data is left: {}\n{log}",
            data.display()
        );
    }
    for never in [
        "its stopped process did not go within 20 s",
        "reload: the previous process did not stop within 20 s",
    ] {
        assert!(!log.contains(never), "{never}: {log}");
    }
}

/// Quit (`lazy_quit`, then each plugin dropped with the host): own
/// processes are stopped one after another, each given its 300 ms grace.
/// Six plugins whose cleanup never ends make the quit about 6 脳 300 ms
/// longer than with none (stopped side by side it would be about 300 ms).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn quit_stops_own_processes_one_after_another() {
    const N: usize = 6;
    let run = |plugins: usize| {
        let mut root = Root::new();
        for n in 0..plugins {
            let id = format!("aa.fixture-quit-{n}");
            root.install(n + 1, &id, &fixture(&id, "1.0.0", Cleanup::Endless, 0, ""));
        }
        root.run_timed(4, &[("VESTRA_SHARED", "0"), ("VESTRA_LAZY", "0")])
    };
    let (_, empty) = run(0);
    let (log, endless) = run(N);
    assert_eq!(log.matches("1.0.0 stopping for quit").count(), N, "{log}");
    let extra = endless.saturating_sub(empty);
    assert!(
        extra >= std::time::Duration::from_millis(1_500),
        "quit took {endless:?} with {N} endless cleanups, {empty:?} with none: \
         not one after another?\n{log}"
    );
}

const SLOW: &str = "aa.fixture-slow";
const BROKEN: &str = "aa.fixture-broken";
const OFF: &str = "aa.fixture-off";

/// `packages.list` states of installed plugins: `starting` while the
/// activation runs, `failed` with the reason when it failed, `disabled`
/// for one the user switched off; `running` once the slow one is ready.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn packages_list_shows_starting_failed_and_disabled() {
    let mut root = Root::new();
    root.install(1, SLOW, &fixture(SLOW, "1.0.0", Cleanup::Quick, 5000, ""));
    root.install(
        2,
        BROKEN,
        "export default function activate() { throw new Error(\"boom\"); }\n",
    );
    root.install(3, OFF, &tracked(OFF, "1.0.0"));
    root.catalog["disabled"] = json!([OFF]);
    root.builtin(
        MANAGER,
        &["plugins.manage"],
        &format!(
            "const show = async (when) => {{\n\
             for (const e of (await plugin.packages.list()).filter((e) => e.id.startsWith(\"aa.\")))\n\
             console.error(`{MANAGER} ${{when}} ${{e.id}} ${{e.state}} ${{e.process}} ${{JSON.stringify(e.error)}}`);\n\
             }};\n\
             let plugin;\n\
             export default function activate(p) {{\n\
             plugin = p;\n\
             setTimeout(() => show(\"early\"), 2000);\n\
             setTimeout(() => show(\"late\"), 8000);\n}}\n"
        ),
    );
    let log = root.run(10, &[("VESTRA_SHARED", "0"), ("VESTRA_LAZY", "0")]);
    for expected in [
        format!("{MANAGER} early {SLOW} starting "),
        format!("{MANAGER} early {OFF} disabled "),
        format!("{MANAGER} early {BROKEN} failed "),
        format!("{MANAGER} late {SLOW} running running null"),
        format!("{MANAGER} late {BROKEN} failed "),
    ] {
        at(&log, &expected);
    }
    let failed = &log[at(&log, &format!("{MANAGER} early {BROKEN} failed "))..];
    let failed = failed.lines().next().unwrap_or_default();
    assert!(failed.contains("boom"), "the reason is listed: {failed}");
    assert!(!log.contains(&format!("{OFF} 1.0.0 start")), "{log}");
}
