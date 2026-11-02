//! Lazy activation with the real host, headless (doc/process/rounds/
//! 2026-10-lazy-activation.md §5.1, step A1): a trivial built-in that only
//! adds a menu command is parked when idle, replayed without a process at
//! the next start, woken by `commands.execute`, and parked again; step B1:
//! a plugin with only the SDK's mirror subscriptions is parked, one with a
//! listener is resident and launched, first runs go eight at a time. Built-ins
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
    package(
        root,
        ID,
        &["commands"],
        format!(
            "export default async function activate(plugin) {{\n\
             const n = ((await plugin.storage.session.get(\"starts\")) ?? 0) + 1;\n\
             await plugin.storage.session.set(\"starts\", n);\n\
             console.error(`{ID} start ${{n}}`);\n\
             plugin.commands.add({{ id: \"hello\", path: [\"Tools\", \"Hello\"],\n\
             run: () => console.error(`{ID} hello in start ${{n}}`) }});\n\
             plugin.onDispose((info) => new Promise((done) => setTimeout(() => {{\n\
             console.error(`{ID} disposed ${{n}} for ${{info?.reason}}`); done(); }}, 1200)));\n}}\n"
        ),
    )
}

/// A built-in package `id` with `capabilities` and `main.js`; its
/// `builtin.json` entry.
fn package(root: &Path, id: &str, capabilities: &[&str], main: String) -> serde_json::Value {
    let files = [
        (
            "manifest.json",
            serde_json::json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1",
                "entry":"main.js","capabilities":capabilities})
            .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main),
    ];
    let directory = root.join(id);
    std::fs::create_dir_all(&directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    serde_json::json!({"id":id,"version":"1.0.0","directory":id,"digest":digest(&files)})
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

/// The recordings under `root`, as JSON.
fn recordings(root: &Path) -> Vec<serde_json::Value> {
    std::fs::read_dir(root.join("store").join("activation"))
        .map(|entries| {
            entries
                .filter_map(|entry| std::fs::read(entry.ok()?.path()).ok())
                .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
                .collect()
        })
        .unwrap_or_default()
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
        // A park is an idle stop (`runtime.stop` reason `idle`, player 0.6.4).
        let disposed = second
            .find(&format!("{ID} disposed 1 for idle"))
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

const MIRROR: &str = "vestra.fixture-mirror";
const LISTENER: &str = "vestra.fixture-listener";

/// Step B1 (§3.1, §3.4): a plugin whose event subscriptions are all the
/// SDK's mirrors (`plugin.player.state`) is classified lazy at its first
/// ready and parked at once, replayed at the next start and woken by its
/// command, which reads the player's state; a plugin with a listener
/// (`player.onChange`) is resident, recorded so at its ready, and started
/// at launch the next time instead of being replayed.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn mirror_subscriptions_park_and_wake_a_listener_keeps_a_plugin_resident() {
    for shared in ["1", "0"] {
        let root = root_with(|plugins| {
            vec![
                package(
                    plugins,
                    MIRROR,
                    &["commands", "events", "playback.read"],
                    format!(
                        "export default async function activate(plugin) {{\n\
                         console.error(`{MIRROR} start`);\n\
                         plugin.commands.add({{ id: \"state\", path: [\"Tools\", \"State\"],\n\
                         run: () => console.error(`{MIRROR} state ${{plugin.player.state.phase}}`) }});\n}}\n"
                    ),
                ),
                package(
                    plugins,
                    LISTENER,
                    &["commands", "events", "playback.read"],
                    format!(
                        "export default async function activate(plugin) {{\n\
                         console.error(`{LISTENER} start`);\n\
                         plugin.player.onChange(() => {{}});\n\
                         plugin.commands.add({{ id: \"hello\", path: [\"Tools\", \"Hello\"], run: () => {{}} }});\n}}\n"
                    ),
                ),
            ]
        });
        let lazy = [
            ("VESTRA_SHARED", shared),
            ("VESTRA_LAZY", "1"),
            ("VESTRA_IDLE_MS", "500"),
        ];
        let first = run(root.path(), 5, &lazy);
        // First runs: the mirror plugin is parked at once, without waiting
        // for an idle time; the listener stays.
        for expected in [
            format!("lazy: {MIRROR} starts for its first run"),
            format!("lazy: {MIRROR} classified lazy"),
            format!("lazy: {MIRROR} parked (1 recorded calls)"),
            format!("lazy: {LISTENER} classified resident: an event listener"),
        ] {
            assert!(first.contains(&expected), "{expected}: {first}");
        }
        assert!(
            !first.contains(&format!("lazy: {LISTENER} parked")),
            "{first}"
        );
        let recorded = recordings(root.path());
        assert_eq!(recorded.len(), 2, "{recorded:?}");
        let resident = recorded
            .iter()
            .filter(|r| r["residentAtReady"] == true && r["calls"] == serde_json::json!([]))
            .count();
        assert_eq!(resident, 1, "one resident recording: {recorded:?}");

        let mut woken = lazy.to_vec();
        woken.push(("VESTRA_TEST_EXECUTE", "vestra.fixture-mirror/state@2500"));
        let second = run(root.path(), 6, &woken);
        for expected in [
            format!("lazy: {MIRROR} replayed (1 calls)"),
            format!("lazy: {LISTENER} starts: resident at its last ready"),
            format!("{LISTENER} start"),
            format!("lazy: {MIRROR} woken by commands.execute"),
            // The SDK read the player's state when it started.
            format!("{MIRROR} state empty"),
        ] {
            assert!(second.contains(&expected), "{expected}: {second}");
        }
        assert!(
            !second.contains(&format!("lazy: {LISTENER} replayed")),
            "{second}"
        );
        let execute = second.find("VESTRA_TEST_EXECUTE").unwrap();
        let started = second.find(&format!("{MIRROR} start")).unwrap();
        assert!(execute < started, "not started before its use: {second}");
        let ran = second.find(&format!("{MIRROR} state empty")).unwrap();
        let parked = second
            .rfind(&format!("lazy: {MIRROR} parked"))
            .unwrap_or_else(|| panic!("parked again: {second}"));
        assert!(ran < parked, "{second}");
    }
}

/// §2.4: on a fresh profile at most eight first runs are in flight; a lazy
/// one frees its place once parked and gone, and the queue goes on.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn first_runs_start_eight_at_a_time() {
    let ids = (0..10)
        .map(|n| format!("vestra.fixture-first-{n}"))
        .collect::<Vec<_>>();
    let root = root_with(|plugins| {
        ids.iter()
            .map(|id| {
                package(
                    plugins,
                    id,
                    &["commands"],
                    format!(
                        "export default function activate(plugin) {{\n\
                         plugin.commands.add({{ id: \"go\", path: [\"Tools\", \"{id}\"], run: () => {{}} }});\n}}\n"
                    ),
                )
            })
            .collect()
    });
    let log = run(
        root.path(),
        8,
        &[
            ("VESTRA_SHARED", "1"),
            ("VESTRA_LAZY", "1"),
            ("VESTRA_IDLE_MS", "60000"),
        ],
    );
    let starts = log
        .match_indices("starts for its first run")
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 10, "{log}");
    let first_park = log
        .find("parked (1 recorded calls)")
        .unwrap_or_else(|| panic!("{log}"));
    assert!(starts[7] < first_park && first_park < starts[8], "{log}");
    for id in &ids {
        let parked = format!("lazy: {id} parked (1 recorded calls)");
        assert!(log.contains(&parked), "{parked}: {log}");
    }
    assert_eq!(recordings(root.path()).len(), 10);
}

/// Installs `id` in the library under `store` as a package from a file:
/// its files in `store/packages/<digest>`, its record in `catalog.json`
/// (`installed`, the rest left empty). `n` makes its instance GUID.
fn install(store: &Path, catalog: &mut serde_json::Value, n: usize, id: &str, main: String) {
    let manifest = serde_json::json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1",
        "entry":"main.js","capabilities":["source.resolve"]});
    let files = [
        ("manifest.json", manifest.to_string()),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main),
    ];
    let digest = digest(&files);
    let directory = store.join("packages").join(&digest);
    std::fs::create_dir_all(&directory).unwrap();
    for (name, text) in &files {
        std::fs::write(directory.join(name), text).unwrap();
    }
    catalog["installed"][id] = serde_json::json!({"revision":1,
        "current":{"digest":digest,"manifest":manifest,"directory":directory},
        "previous":null,
        "instance":{"guid":format!("00000000-0000-4000-8000-{n:012x}"),
            "source":{"kind":"file"},"installedAt":1}});
}

/// Review M (B1): first runs of other plugins never take the places while a
/// built-in waits for its first run. Their mounts, sequence and resolvers
/// wait for the built-ins (`builtins_starting`, not headless), so nine
/// installed plugins whose ids sort before `vestra.` and register a
/// resolver at activation must not start before the built-in, or all
/// eight places would wait on it until their activation timed out. Headless
/// does not hold those requests, so the order is what is checked: the
/// built-in starts first, all ten run, none times out, all are parked.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn built_in_first_runs_go_before_other_plugins() {
    let root = root_with(|plugins| vec![fixture(plugins)]);
    let store = root.path().join("store");
    let mut catalog =
        serde_json::json!({"installed":{},"pending":{},"operations":{},"lastCommit":{}});
    let ids = (0..9)
        .map(|n| format!("aa.fixture-{n}"))
        .collect::<Vec<_>>();
    for (n, id) in ids.iter().enumerate() {
        install(
            &store,
            &mut catalog,
            n + 1,
            id,
            format!(
                "export default function activate(plugin) {{\n\
                 plugin.resolvers.add({{ id: \"r\", schemes: [\"https\"], hosts: [\"{id}.example\"],\n\
                 resolve: async () => {{ throw new Error(\"no\"); }} }});\n}}\n"
            ),
        );
    }
    std::fs::write(store.join("catalog.json"), catalog.to_string()).unwrap();
    let log = run(
        root.path(),
        10,
        &[
            ("VESTRA_SHARED", "1"),
            ("VESTRA_LAZY", "1"),
            ("VESTRA_IDLE_MS", "60000"),
        ],
    );
    let builtin = log
        .find(&format!("lazy: {ID} starts for its first run"))
        .unwrap_or_else(|| panic!("{log}"));
    for id in &ids {
        let started = log
            .find(&format!("lazy: {id} starts for its first run"))
            .unwrap_or_else(|| panic!("{id}: {log}"));
        assert!(builtin < started, "{id} before the built-in: {log}");
        let parked = format!("lazy: {id} parked (1 recorded calls)");
        assert!(log.contains(&parked), "{parked}: {log}");
    }
    assert!(!log.contains("timed out"), "{log}");
    assert_eq!(recordings(root.path()).len(), 10);
}

const WHEN: &str = "vestra.fixture-when";
const OPENER: &str = "vestra.fixture-opener";

/// §6.9 (B2): a command whose `enabled` is a `plugin.when` condition is
/// evaluated by the host, so its plugin holds nothing and is parked, and
/// its state follows playback while it is parked: disabled (with its
/// reason, no wake) before a file is open, enabled once it is ready, when
/// its use wakes it. The condition is recorded and replayed with the
/// command. A resident fixture opens a generated audio file (ffmpeg on
/// `PATH` or `VESTRA_FFMPEG`, as for ci:local).
#[test]
#[ignore = "needs Node 26, libmpv and ffmpeg: run with --include-ignored (ci:local)"]
fn a_when_command_is_parked_and_its_state_follows_playback() {
    let media = tempfile::tempdir().unwrap();
    let audio = media.path().join("tone.wav");
    let ffmpeg = std::env::var("VESTRA_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let made = Command::new(ffmpeg)
        .args(["-v", "error", "-f", "lavfi", "-i", "sine=duration=20", "-y"])
        .arg(&audio)
        .status()
        .unwrap();
    assert!(made.success());
    let path = serde_json::to_string(&audio.to_string_lossy()).unwrap();
    let root = root_with(|plugins| {
        vec![
            package(
                plugins,
                WHEN,
                &["commands"],
                format!(
                    "export default function activate(plugin) {{\n\
                     console.error(`{WHEN} start`);\n\
                     plugin.commands.add({{ id: \"save\", path: [\"Tools\", \"Save\"],\n\
                     enabled: plugin.when.ready, reason: \"Nothing open\",\n\
                     run: () => console.error(`{WHEN} saved`) }});\n}}\n"
                ),
            ),
            package(
                plugins,
                OPENER,
                &["events", "playback.read", "playback.control"],
                format!(
                    "export default function activate(plugin) {{\n\
                     plugin.player.onChange(() => {{}});\n\
                     setTimeout(() => plugin.player.open({path}).then(\n\
                     () => console.error(`{OPENER} opened`),\n\
                     (error) => console.error(`{OPENER} failed ${{error}}`)), 3500);\n}}\n"
                ),
            ),
        ]
    });
    let lazy = [
        ("VESTRA_SHARED", "1"),
        ("VESTRA_LAZY", "1"),
        ("VESTRA_IDLE_MS", "500"),
    ];
    let mut executes = lazy.to_vec();
    executes.push((
        "VESTRA_TEST_EXECUTE",
        "vestra.fixture-when/save@2500,vestra.fixture-when/save@7500",
    ));
    let first = run(root.path(), 10, &executes);
    let parked = first
        .find(&format!("lazy: {WHEN} parked (1 recorded calls)"))
        .unwrap_or_else(|| panic!("{first}"));
    // Parked, and still disabled with its reason: the use wakes nothing.
    let refused = first
        .find("lazy: VESTRA_TEST_EXECUTE: command disabled: Nothing open")
        .unwrap_or_else(|| panic!("{first}"));
    assert!(parked < refused, "{first}");
    let opened = first
        .find(&format!("{OPENER} opened"))
        .unwrap_or_else(|| panic!("{first}"));
    let woken = first
        .find(&format!("lazy: {WHEN} woken by commands.execute"))
        .unwrap_or_else(|| panic!("enabled once ready, while parked: {first}"));
    assert!(refused < opened && opened < woken, "{first}");
    assert_eq!(count(&first, &format!("] {WHEN} start")), 2, "{first}");
    assert!(first.contains(&format!("{WHEN} saved")), "{first}");
    let reason = format!(
        "lazy: {OPENER} classified resident: an event listener; listens to source.state: a listener added at main.js:"
    );
    assert!(first.contains(&reason), "the reason names where: {first}");
    let recorded = recordings(root.path());
    let save = recorded
        .iter()
        .flat_map(|r| r["calls"].as_array().cloned().unwrap_or_default())
        .find(|call| call["params"]["id"] == "save")
        .unwrap_or_else(|| panic!("{recorded:?}"));
    assert_eq!(
        save["params"]["when"],
        serde_json::json!({"enabled":{"phase":["ready"]}})
    );

    // Replayed: the recorded condition is evaluated without a process.
    let mut replayed = lazy.to_vec();
    replayed.push(("VESTRA_TEST_EXECUTE", "vestra.fixture-when/save@2500"));
    let second = run(root.path(), 4, &replayed);
    assert!(
        second.contains(&format!("lazy: {WHEN} replayed (1 calls)")),
        "{second}"
    );
    assert!(
        second.contains("lazy: VESTRA_TEST_EXECUTE: command disabled: Nothing open"),
        "{second}"
    );
    assert!(!second.contains(&format!("] {WHEN} start")), "{second}");
}

const PACKAGES: &str = "vestra.fixture-packages";
const CHANGER: &str = "vestra.fixture-changer";

/// §6.9 (B2): a listener of `packages.changed` (a wake event) does not keep
/// its plugin running: it is parked with its subscription recorded, and a
/// package change wakes it to receive the event. On the next start it is
/// replayed, not launched.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_packages_listener_is_parked_and_woken_by_a_package_change() {
    let root = root_with(|plugins| {
        vec![
            package(
                plugins,
                PACKAGES,
                &["commands"],
                format!(
                    "export default function activate(plugin) {{\n\
                     console.error(`{PACKAGES} start`);\n\
                     plugin.packages.onChange((list) => console.error(`{PACKAGES} heard ${{\n\
                     list.filter((p) => p.id === \"aa.fixture-target\").map((p) => p.state)}}`));\n}}\n"
                ),
            ),
            package(
                plugins,
                CHANGER,
                &["events", "playback.read", "plugins.manage"],
                format!(
                    "export default function activate(plugin) {{\n\
                     plugin.player.onChange(() => {{}});\n\
                     setTimeout(() => plugin.packages.setEnabled(\"aa.fixture-target\", false).then(\n\
                     () => console.error(`{CHANGER} disabled`)), 3000);\n}}\n"
                ),
            ),
        ]
    });
    let store = root.path().join("store");
    let mut catalog =
        serde_json::json!({"installed":{},"pending":{},"operations":{},"lastCommit":{}});
    install(
        &store,
        &mut catalog,
        1,
        "aa.fixture-target",
        "export default function activate() {}\n".into(),
    );
    std::fs::write(store.join("catalog.json"), catalog.to_string()).unwrap();
    let lazy = [
        ("VESTRA_SHARED", "1"),
        ("VESTRA_LAZY", "1"),
        ("VESTRA_IDLE_MS", "500"),
    ];
    let first = run(root.path(), 7, &lazy);
    for expected in [
        format!("lazy: {PACKAGES} classified lazy"),
        format!("lazy: {PACKAGES} parked (1 recorded calls)"),
        format!("{CHANGER} disabled"),
        format!("lazy: {PACKAGES} woken by packages.changed"),
        format!("{PACKAGES} heard disabled"),
    ] {
        assert!(first.contains(&expected), "{expected}: {first}");
    }
    let parked = first.find(&format!("lazy: {PACKAGES} parked")).unwrap();
    let woken = first
        .find(&format!("lazy: {PACKAGES} woken by packages.changed"))
        .unwrap();
    assert!(parked < woken, "{first}");
    // Woken for each change: the target's own start, then its disable.
    assert!(
        count(&first, &format!("] {PACKAGES} start")) >= 2,
        "{first}"
    );
    // Parked again once idle after the event.
    assert!(
        first[woken..].contains(&format!("lazy: {PACKAGES} parked")),
        "{first}"
    );
    let subscribed =
        serde_json::json!([{"method":"events.subscribe","params":{"event":"packages.changed"}}]);
    let recorded = recordings(root.path());
    assert!(
        recorded.iter().any(|r| r["calls"] == subscribed),
        "{recorded:?}"
    );

    // The next start replays it; nothing changes, nothing wakes it.
    let second = run(root.path(), 3, &lazy);
    assert!(
        second.contains(&format!("lazy: {PACKAGES} replayed (1 calls)")),
        "{second}"
    );
    assert!(!second.contains(&format!("] {PACKAGES} start")), "{second}");
}
