//! Characterization tests of the plugin lifecycle with the real host,
//! headless, written before the plugin process supervisor
//! (doc/architecture/supervisor.md) took over, so that the move could show
//! zero behaviour difference; updated where the supervisor changed
//! behaviour on purpose. They pin what the other suites do not:
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
//! | A cleanup that never ends: reload (shared runner) and upgrade (own process) wait only for the stop's grace; `busy` while an operation runs | here: `an_endless_cleanup_holds_a_reload_or_an_upgrade_only_for_its_grace` |
//! | Standby with a process still stopping: wake waits, enable goes through `packages.reloads` (`starting`, `busy`), uninstall removes the data after the reap (or at once once reaped) | here: `standby_holds_wake_enable_and_uninstall_until_the_parked_process_is_gone` |
//! | Quit stops own processes one after another (300 ms each) | here: `quit_stops_own_processes_one_after_another` |
//! | `packages.list` `starting`, `failed` (with its reason), `disabled` | here: `packages_list_shows_starting_failed_and_disabled` |
//! | Book entry leaked on a failed spawn, since fixed | here: `b2a_a_failed_spawn_gives_its_activation_place_back` |
//! | `Host.cleanup` only grew, since fixed | here: `b2b_stop_bookkeeping_is_the_units_and_cannot_grow` |
//! | Disable then enable ran two processes, since fixed | here: `b2b_disable_then_enable_waits_for_the_old_process` |
//! | Uninstall then reinstall ran two processes, since fixed | here: `b2b_uninstall_then_reinstall_waits_for_the_old_process` |
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
    /// `builtin` for a `main` that listens to the player (`RESIDENT`): with
    /// the capabilities that needs.
    fn builtin_resident(&mut self, id: &str, capabilities: &[&str], main: &str) {
        let mut capabilities = capabilities.to_vec();
        capabilities.extend(RESIDENT_CAPABILITIES);
        self.builtin(id, &capabilities, main);
    }
    /// Installs third-party `id` as instance `n` (its data directory is
    /// `data/<guid>`).
    fn install(&mut self, n: usize, id: &str, main: &str) -> PathBuf {
        self.install_with(n, id, &[], main)
    }
    /// `install` for a `main` that listens to the player (`RESIDENT`).
    fn install_resident(&mut self, n: usize, id: &str, main: &str) -> PathBuf {
        self.install_with(n, id, &RESIDENT_CAPABILITIES, main)
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

/// A plugin that holds nothing live goes to standby at its first ready.
/// The tests below that need running processes keep their plugins running
/// this way: a listener of the player's state makes a plugin resident, and
/// a resident plugin never stands by (supervisor.md §2). Inside
/// `activate(plugin)`.
const RESIDENT: &str = "plugin.player.onChange(() => {});\n";
/// What `RESIDENT` needs in the manifest.
const RESIDENT_CAPABILITIES: [&str; 2] = ["events", "playback.read"];

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
    let lazy = [("VESTRA_SHARED", "1"), ("VESTRA_IDLE_MS", "500")];
    // A first run: lazy at its ready, parked once quiet, recorded.
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
/// data directory (`alive-<pid>`; the directory is created first, and one
/// that cannot be read counts as no overlap). `overlaps [${others}]` is `[]` exactly
/// when no earlier process of the plugin still runs (own processes only:
/// shared-runner members share one pid and get no `VESTRA_PLUGIN_DATA_DIR`:
/// always `[]` there).
const ALIVE: &str = "import { readdirSync as aliveList, writeFileSync as aliveNote, mkdirSync as aliveMake } from \"node:fs\";\n\
const aliveDir = process.env.VESTRA_PLUGIN_DATA_DIR;\n\
const alive = (pid) => { try { process.kill(pid, 0); return true; } catch { return false; } };\n\
const aliveRead = () => { try { aliveMake(aliveDir, { recursive: true }); return aliveList(aliveDir); } catch { return []; } };\n\
const others = !aliveDir ? [] : aliveRead().filter((f) => f.startsWith(\"alive-\"))\n\
.map((f) => Number(f.slice(6))).filter((pid) => pid !== process.pid && alive(pid));\n\
if (aliveDir) try { aliveNote(`${aliveDir}/alive-${process.pid}`, \"\"); } catch {}\n";

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

/// `fixture` with a quick cleanup and no delay, resident (`RESIDENT`, so
/// that it stays running under on-demand start; installed with
/// `install_resident`).
fn tracked(id: &str, version: &str) -> String {
    fixture(id, version, Cleanup::Quick, 0, RESIDENT)
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
    root.install_resident(1, RELOAD, &tracked(RELOAD, "1.0.0"));
    root.install_resident(2, UPGRADE, &tracked(UPGRADE, "1.0.0"));
    root.install_resident(3, TOGGLE, &tracked(TOGGLE, "1.0.0"));
    let gone_data = root.install_resident(4, GONE, &tracked(GONE, "1.0.0"));
    let candidate = root.path().join("candidates").join(UPGRADE);
    package(
        &candidate,
        UPGRADE,
        "2.0.0",
        &RESIDENT_CAPABILITIES,
        &tracked(UPGRADE, "2.0.0"),
    );
    let candidate = serde_json::to_string(&candidate.to_string_lossy()).unwrap();
    root.builtin(
        "vestra.fixture-calm",
        &[],
        "export default function activate() {}\n",
    );
    root.builtin_resident(
        MANAGER,
        &["plugins.manage"],
        &format!(
            "const log = (text) => console.error(`{MANAGER} ${{text}}`);\n\
             const code = (promise) => promise.then(() => \"ok\", (e) => e.code);\n\
             const sleep = (ms) => new Promise((done) => setTimeout(done, ms));\n\
             export default function activate(plugin) {{\n\
             {RESIDENT}\
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
    // The plugins are resident, so they keep running.
    let log = root.run(14, &[("VESTRA_SHARED", "0")]);
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
        // cleanup ends within the grace either way: 5 s, 300 ms before on-demand start); a reload and an
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
             const key = (list) => JSON.stringify(list.map((p) => ({{ ...p, process: undefined }})));\n\
             let last = null;\n\
             plugin.packages.onChange((list) => {{ const now = key(list); console.error(`{WATCH} changed ${{shown(list)}} same=${{now === last}}`); last = now; }});\n\
             setTimeout(async () => console.error(`{WATCH} listed ${{shown(await plugin.packages.list())}}`), 3500);\n}}\n"
        ),
    );
    let log = root.run(5, &[("VESTRA_SHARED", "1"), ("VESTRA_IDLE_MS", "500")]);
    let parked = at(&log, &format!("lazy: {IDLE} parked"));
    let listed = at(&log, &format!("{WATCH} listed running idle"));
    assert!(parked < listed, "{log}");
    at(&log, &format!("lazy: {WATCH} classified resident"));
    // Every announcement changes something other than `process`: the park
    // (`process` running → idle) is never announced by itself. (Counting
    // the announcements that show it `running` raced with another plugin's
    // change announced after the park: about 1 run in 30.)
    assert!(log.contains(&format!("{WATCH} changed")), "{log}");
    assert!(!log.contains("same=true"), "the park was announced: {log}");
}

/// A bug these tests pinned (an activation place leaked), since fixed.
/// `load_package` takes an activation place (`activation_begin`) before
/// it spawns; before the supervisor an early return (`Plugin::load_manifest(..)?`)
/// never gave it back, so after 64 failed spawns every later start failed
/// with "activation tracking limit reached" instead of its own error. Now a
/// failed spawn ends its activation (and the activation is its unit's, one
/// per unit): the 65th fails to spawn Node like the others. Node is pointed
/// at a missing file, so no plugin process runs at all. With the spawn
/// retry (supervisor.md §6) each start is tried three more times (250 ms × 2ⁿ)
/// before it is listed failed, so the failures end in any order.
#[test]
#[ignore = "needs libmpv: run with --include-ignored (ci:local)"]
fn b2a_a_failed_spawn_gives_its_activation_place_back() {
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
    let log = root.run(6, &[("VESTRA_SHARED", "0"), ("VESTRA_NODE", node.as_ref())]);
    let failures = log
        .lines()
        .filter(|line| line.contains("failed to start: "))
        .collect::<Vec<_>>();
    assert_eq!(failures.len(), 65, "{log}");
    // Every one fails to spawn Node after its three retries: none
    // finds the places taken by the failures before it.
    for n in 0..65 {
        let id = format!("aa.fixture-{n:03}");
        assert!(
            failures
                .iter()
                .any(|line| line.contains(&format!("{id} failed to start: start Node"))),
            "{id}\n{log}"
        );
        let retries = log
            .lines()
            .filter(|line| line.contains(&format!("{id} could not start, tried again shortly")))
            .count();
        assert_eq!(retries, 3, "{id}\n{log}");
    }
    // They start in id order: their first tries.
    let mut firsts = Vec::<String>::new();
    for line in log.lines() {
        if let Some(rest) = line.strip_prefix("supervisor: ")
            && rest.contains("could not start, tried again shortly")
            && let Some(id) = rest.split_whitespace().next()
            && !firsts.iter().any(|seen| seen == id)
        {
            firsts.push(id.to_owned());
        }
    }
    let ordered = (0..65)
        .map(|n| format!("aa.fixture-{n:03}"))
        .collect::<Vec<_>>();
    assert_eq!(firsts, ordered, "{log}");
    assert!(!log.contains("activation tracking limit reached"), "{log}");
}

/// A bug these tests pinned (A18, part 2), since fixed (supervisor.md §3).
/// `Host.cleanup` (whether a stopped owner's process is reaped and the
/// playback worker cleaned up after it) got an entry at every stop and none
/// left it in a run: no code removed one. The supervisor replaces it by the unit's
/// `Stopping` (the supervisor's `reap.rs`): one per unit, set at a stop,
/// cleared when that process is reaped and cleaned up after, so it cannot
/// grow (the supervisor's `stop_bookkeeping_is_one_per_unit_and_goes`
/// runs a thousand stops through it). No headless output shows its size, so
/// the source is read: no receiver's `.cleanup` field is used any more
/// (the manager's `packages.cleanup`, a library thread, aside), and no
/// use removes entries from or replaces such a map either.
#[test]
fn b2b_stop_bookkeeping_is_the_units_and_cannot_grow() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let uses = cleanup_uses(&source);
    assert!(uses.is_empty(), "`Host.cleanup` is back: {uses:?}");
    let removals = cleanup_removals(&source);
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
         mem::swap(&mut self.cleanup, &mut m); self.cleanup == other;\n",
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
            "replace",
            "swap"
        ],
        "{found:?}"
    );
}

/// The `.rs` files under `root` whose code (comments aside) uses a
/// `.cleanup` field other than `packages.cleanup` at all (`cleanup_done`,
/// `cleanup_tick`, `cleanup_queue` are other names).
fn cleanup_uses(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            // Code only: comments name the old field.
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .lines()
                .map(|line| line.split("//").next().unwrap_or_default())
                .collect::<String>()
                .split_whitespace()
                .collect::<String>();
            let used = text.match_indices(".cleanup").any(|(at, _)| {
                let after = &text[at + ".cleanup".len()..];
                !after
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
                    && !text[..at].ends_with("packages")
            });
            if used {
                found.push(path);
            }
        }
    }
    found.sort();
    found
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
                        ["take", "replace", "swap"]
                            .into_iter()
                            .find(|f| head.ends_with(&format!("mem::{f}")))
                            .map(str::to_owned)
                    })
                } else if after.starts_with(',') {
                    let receiver = before.rfind("(&mut").map(|i| &before[..i]);
                    receiver.and_then(|head| {
                        ["replace", "swap"]
                            .into_iter()
                            .find(|f| head.ends_with(&format!("mem::{f}")))
                            .map(str::to_owned)
                    })
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

/// A bug these tests pinned (disable then enable ran two processes), since
/// fixed. `packages.setEnabled(false)` then
/// `setEnabled(true)` on a running plugin: before the supervisor the stop took it out
/// of `plugins` at once and `parked_process` looked only at parked
/// processes, so the enable started a new process on the same data
/// directory while the old one was still in its 300 ms grace (here it never
/// finishes its cleanup, so it is killed at the end of it; 2026-11-02,
/// Windows laptop: all three toggles did). Now the enable is a job that
/// waits for the old process to be reaped and cleaned up after. Each
/// instance notes its pid in its data directory when its module loads and
/// logs the other instances' processes still alive then: `overlaps []`
/// every time.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn b2b_disable_then_enable_waits_for_the_old_process() {
    let mut root = Root::new();
    // Both resident (they stay running, as with on-demand start off
    // before), and 6 s between toggles: a disable now gives the endless
    // cleanup 5 s (the stop grace, was 300 ms) before it is killed, and a
    // toggle while the last one's enable still waits would merge with it.
    root.install_resident(
        1,
        TWICE,
        &format!(
            "{ALIVE}console.error(`{TWICE} pid ${{process.pid}} overlaps [${{others}}]`);\n\
             export default function activate(plugin) {{\n\
             {RESIDENT}\
             plugin.onDispose(() => new Promise(() => {{}}));\n}}\n"
        ),
    );
    root.builtin_resident(
        MANAGER,
        &["plugins.manage"],
        &format!(
            "const sleep = (ms) => new Promise((done) => setTimeout(done, ms));\n\
             export default function activate(plugin) {{\n\
             {RESIDENT}\
             setTimeout(async () => {{\n\
             for (let i = 0; i < 3; i++) {{\n\
             const off = plugin.packages.setEnabled(\"{TWICE}\", false);\n\
             const on = plugin.packages.setEnabled(\"{TWICE}\", true);\n\
             const codes = await Promise.all([off, on].map((p) => p.then(() => \"ok\", (e) => e.code)));\n\
             console.error(`{MANAGER} toggled ${{codes}}`);\n\
             await sleep(6000);\n\
             }}\n\
             console.error(`{MANAGER} done`);\n\
             }}, 1500);\n}}\n"
        ),
    );
    let log = root.run(24, &[("VESTRA_SHARED", "0")]);
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
    assert_eq!(
        overlapping, 0,
        "an enable started beside the old process: {log}"
    );
}

const AGAIN: &str = "aa.fixture-again";

/// The same class of bug (found in review):
/// `packages.uninstall` without `removeData` of a running (not parked)
/// plugin, then an install of it again at once. The uninstall keeps the
/// instance (dormant), so the reinstall reconnects to the same data
/// directory. Before the supervisor the install's operation waited only for a running
/// or parked version (`Operation.waiting`), not for the uninstalled one's
/// process still in its 300 ms grace (its cleanup here never ends), so the
/// new instance could start beside it. Through the SDK the race is narrow:
/// on main 767fad55 (2026-11-02, Windows laptop) the reinstalled module
/// loaded 320–330 ms after the uninstall, just after the old process was
/// killed (0 of 3 overlapped), so this run does not show the old behaviour
/// here; the supervisor's `an_install_after_an_uninstall_waits_for_its_process`
/// pins the wait itself. Now
/// the operation's start is a job that waits for that process to be reaped
/// and cleaned up after: `overlaps []` every time, and every uninstall and
/// install is committed.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn b2b_uninstall_then_reinstall_waits_for_the_old_process() {
    let mut root = Root::new();
    let main = format!(
        "{ALIVE}console.error(`{AGAIN} pid ${{process.pid}} overlaps [${{others}}]`);\n\
         export default function activate(plugin) {{\n\
         {RESIDENT}\
         plugin.onDispose(() => new Promise(() => {{}}));\n}}\n"
    );
    // Resident (it stays running, as with on-demand start off
    // before); each uninstall now gives the endless cleanup 5 s
    // (the stop grace, was 300 ms) before the kill, so the run is longer.
    root.install_resident(1, AGAIN, &main);
    // A digest of its own each time: the uninstalled one's directory may
    // still be being removed by the library thread (an install of the same
    // digest would be `busy` then).
    let candidates = (0..3)
        .map(|n| {
            let candidate = root.path().join("candidates").join(format!("{AGAIN}-{n}"));
            package(
                &candidate,
                AGAIN,
                "1.0.0",
                &RESIDENT_CAPABILITIES,
                &format!("{main}// {n}\n"),
            );
            js_path(&candidate)
        })
        .collect::<Vec<_>>()
        .join(", ");
    root.builtin_resident(
        MANAGER,
        &["plugins.manage"],
        &format!(
            "const sleep = (ms) => new Promise((done) => setTimeout(done, ms));\n\
             const log = (text) => console.error(`{MANAGER} ${{text}}`);\n\
             export default function activate(plugin) {{\n\
             {RESIDENT}\
             const p = plugin.packages;\n\
             const candidates = [{candidates}];\n\
             setTimeout(async () => {{\n\
             for (let i = 0; i < 3; i++) {{\n\
             try {{\n\
             const removal = await p.uninstall(\"{AGAIN}\", {{ removeData: false }});\n\
             const update = await (await p.validate(candidates[i])).install();\n\
             log(`again ${{await removal.done}} ${{await update.done}}`);\n\
             }} catch (e) {{ log(`failed ${{e.code}} ${{e.message}}`); }}\n\
             await sleep(1500);\n\
             }}\n\
             log(\"done\");\n\
             }}, 1500);\n}}\n"
        ),
    );
    let log = root.run(28, &[("VESTRA_SHARED", "0")]);
    at(&log, &format!("{MANAGER} done"));
    assert_eq!(
        log.matches(&format!("{MANAGER} again committed committed"))
            .count(),
        3,
        "{log}"
    );
    let starts = log
        .lines()
        .filter(|line| line.contains(&format!("{AGAIN} pid ")))
        .collect::<Vec<_>>();
    assert_eq!(
        starts.len(),
        4,
        "the first start and three reinstalls: {log}"
    );
    let overlapping = starts
        .iter()
        .filter(|line| !line.ends_with("overlaps []"))
        .count();
    assert_eq!(
        overlapping, 0,
        "a reinstall started beside the uninstalled process: {log}"
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
/// `MEMBER_GONE`), and an upgrade of an own process for its grace (the
/// stop grace, 5 s; 300 ms before on-demand start) (preconditions for the
/// 20 s texts staying unreachable: see the end of the test). While the
/// upgrade's operation runs, the plugin's `setEnabled`, `reload` and
/// `uninstall` are `busy` (the operation half of `package_busy`). The
/// plugins are resident (`RESIDENT`) so that they run as they did
/// with on-demand start off.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn an_endless_cleanup_holds_a_reload_or_an_upgrade_only_for_its_grace() {
    let mut root = Root::new();
    root.builtin_resident(
        ENDLESS,
        &[],
        &fixture(ENDLESS, "1.0.0", Cleanup::Endless, 0, RESIDENT),
    );
    root.install_resident(
        1,
        ENDLESS_UP,
        &fixture(ENDLESS_UP, "1.0.0", Cleanup::Endless, 0, RESIDENT),
    );
    let candidate = root.path().join("candidates").join(ENDLESS_UP);
    // Activates for 3 s: the operation is in `activating` meanwhile.
    package(
        &candidate,
        ENDLESS_UP,
        "2.0.0",
        &RESIDENT_CAPABILITIES,
        &fixture(ENDLESS_UP, "2.0.0", Cleanup::Quick, 3000, RESIDENT),
    );
    let candidate = js_path(&candidate);
    root.builtin_resident(
        MANAGER,
        &["plugins.manage"],
        &format!(
            "const log = (text) => console.error(`{MANAGER} ${{text}}`);\n\
             const code = (promise) => promise.then(() => \"ok\", (e) => `${{e.code}}: ${{e.message}}`);\n\
             export default function activate(plugin) {{\n\
             {RESIDENT}\
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
    // Longer than before on-demand start: the upgrade's stop waits the own process's 5 s.
    let log = root.run(18, &[("VESTRA_SHARED", "1")]);
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
    // No assertion on the 20 s failure texts: these runs are shorter than
    // 20 s. They stay unreachable today only while (1) the playback worker
    // handles `owner.disabled`/`owner.parked` promptly (`BackendCleaned` is
    // then sent unconditionally, playback/backend.rs:544) and (2) the reap
    // ends within the stop's grace; `advance_package` times its 20 s from
    // `op.started`, about 17.3 s in the worst case for own processes today,
    // and above 20 s is possible for a shared-runner member (6 s
    // `MEMBER_GONE`).
}

const WAKE: &str = "aa.fixture-wake";
const ENABLE: &str = "aa.fixture-enable";
const FRESH: &str = "aa.fixture-gone-fresh";
const REAPED: &str = "aa.fixture-gone-reaped";

/// Standby (lazy activation) with own processes whose cleanup never ends:
/// a park waits `PARK_WAIT` (5 s; 2 s before on-demand start) for its process, and whatever comes
/// meanwhile waits for that process to go, never beside it: a wake by a
/// command (`Parked.waiting`, `lazy_tick`; the 20 s `STOP_WAIT_MS` never
/// runs out), a disable and enable (`set_enabled(true)` → `packages.reloads`:
/// listed `starting`; a second enable `ok`, formerly `busy`), an
/// uninstall that removes the
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
    // the parked process is gone; then a fresh start (the disable cleared
    // its session values) beside nothing. A second enable meanwhile was
    // `busy` (the old enable-after-park entry in `packages.reloads`); now
    // it is merged into the waiting start (an enable is the user's, not
    // a package RPC).
    at(&log, &format!("lazy: {ENABLE} parked"));
    at(
        &log,
        &format!("{MANAGER} enable off ok on ok then starting"),
    );
    assert!(
        log.contains(" again ok"),
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
    // No assertion on the 20 s failure texts: these runs are shorter than
    // 20 s. They stay unreachable today only while (1) the playback worker
    // handles `owner.disabled`/`owner.parked` promptly (`BackendCleaned` is
    // then sent unconditionally, playback/backend.rs:544) and (2) the reap
    // ends within the stop's grace; `advance_package` times its 20 s from
    // `op.started`, about 17.3 s in the worst case for own processes today,
    // and above 20 s is possible for a shared-runner member (6 s
    // `MEMBER_GONE`).
}

/// Quit. These tests first pinned own processes stopped one after another,
/// each given its 300 ms grace (six plugins whose cleanup never ends made
/// the quit about 6 × 300 ms longer than with none). Now quitting spends
/// at most about 1 s on plugin shutdown (supervisor.md §9): every process gets its
/// stop at once and one deadline of 1 s, so twelve endless cleanups (3.6 s
/// one after another) cost about that 1 s.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn quit_stops_own_processes_side_by_side_within_one_second() {
    const N: usize = 12;
    let run = |plugins: usize| {
        let mut root = Root::new();
        for n in 0..plugins {
            let id = format!("aa.fixture-quit-{n}");
            // Resident, so they run at the quit as they did with
            // on-demand start off (a lazy one is in standby by then).
            root.install_resident(
                n + 1,
                &id,
                &fixture(&id, "1.0.0", Cleanup::Endless, 0, RESIDENT),
            );
        }
        root.run_timed(4, &[("VESTRA_SHARED", "0")])
    };
    // A warm-up run first (a cold start of the host and Node once made the
    // empty baseline as slow as six endless cleanups), then three of each,
    // interleaved, compared by their medians.
    run(0);
    let median = |mut runs: Vec<std::time::Duration>| {
        runs.sort_unstable();
        runs[runs.len() / 2]
    };
    let (mut empty, mut endless, mut log) = (Vec::new(), Vec::new(), String::new());
    for _ in 0..3 {
        empty.push(run(0).1);
        let (stderr, took) = run(N);
        assert_eq!(
            stderr.matches("1.0.0 stopping for quit").count(),
            N,
            "{stderr}"
        );
        endless.push(took);
        log = stderr;
    }
    let (empty, endless) = (median(empty), median(endless));
    let extra = endless.saturating_sub(empty);
    // One deadline for all, about 1 s (before the supervisor: N × 300 ms and more).
    assert!(
        extra < std::time::Duration::from_millis(2_000),
        "quit took {endless:?} (median) with {N} endless cleanups, {empty:?} with none: \
         still one after another?\n{log}"
    );
    let stopped = log
        .lines()
        .find_map(|line| {
            line.split_once(" plugin processes stopped in ")
                .map(|(_, rest)| rest)
        })
        .and_then(|rest| rest.split(' ').next()?.parse::<u64>().ok())
        .unwrap_or_else(|| panic!("no shutdown line\n{log}"));
    assert!((900..1_400).contains(&stopped), "{stopped} ms\n{log}");
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
    // The slow one and the manager are resident (`RESIDENT`): they
    // stay running, so the slow one lists `running running` once ready, as
    // with on-demand start off before (a lazy one would list `running idle`
    // in standby).
    root.install_resident(
        1,
        SLOW,
        &fixture(SLOW, "1.0.0", Cleanup::Quick, 5000, RESIDENT),
    );
    root.install(
        2,
        BROKEN,
        "export default function activate() { throw new Error(\"boom\"); }\n",
    );
    root.install_resident(3, OFF, &tracked(OFF, "1.0.0"));
    root.catalog["disabled"] = json!([OFF]);
    root.builtin_resident(
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
             {RESIDENT}\
             setTimeout(() => show(\"early\"), 2000);\n\
             setTimeout(() => show(\"late\"), 8000);\n}}\n"
        ),
    );
    let log = root.run(10, &[("VESTRA_SHARED", "0")]);
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
