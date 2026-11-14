//! Standby and wake with the real host, headless (doc/architecture/
//! supervisor.md §4): a trivial built-in that only
//! adds a menu command is parked when idle, replayed without a process at
//! the next start, woken by `commands.execute`, and parked again; then
//! a plugin with only the SDK's mirror subscriptions is parked, one with a
//! listener is resident and launched, first runs go eight at a time. Built-ins
//! from a `builtin.json` schema 3 under `VESTRA_BUILTIN_PLUGINS`, the
//! development runner, Node 26 on `PATH` (or `VESTRA_NODE`) and libmpv.
//! Ignored by default; `npm run ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::Path;
use std::process::Command;
#[path = "explicit/mod.rs"]
mod explicit;

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

/// A built-in package `id` with `main.js` (and the calls `capabilities`
/// stand for, `explicit`); its `builtin.json` entry.
fn package(root: &Path, id: &str, capabilities: &[&str], main: String) -> serde_json::Value {
    let files = [
        (
            "manifest.json",
            serde_json::json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1",
                "entry":"main.js"})
            .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", explicit::main(capabilities, &main)),
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
/// parked again. Both with the shared runner and with own processes.
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
        let lazy = [("VESTRA_SHARED", shared), ("VESTRA_IDLE_MS", "500")];

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
    }
}

const MIRROR: &str = "vestra.fixture-mirror";
const LISTENER: &str = "vestra.fixture-listener";

/// A plugin whose event subscriptions are all the
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
        let lazy = [("VESTRA_SHARED", shared), ("VESTRA_IDLE_MS", "500")];
        let first = run(root.path(), 5, &lazy);
        // First runs: the mirror plugin is parked after its quiet period;
        // the listener stays.
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

const TAKES_UP: &str = "vestra.fixture-takes-up";

/// A plugin whose first run holds nothing live is parked; woken later by its
/// command, it takes up something only its process can serve in the playback
/// worker (as Shader Upscale's shaders once turned on; here the video view,
/// which a headless player takes too) and saves that choice. It is resident
/// from then on: the next start launches it, and its activation takes up
/// the resource again, instead of replaying its command without a process
/// (the shaders were not applied until its window opened).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_plugin_that_takes_up_a_live_resource_after_its_first_run_starts_at_the_next_launch() {
    let root = root_with(|plugins| {
        vec![package(
            plugins,
            TAKES_UP,
            &["commands", "processing.video", "playback.read", "events"],
            format!(
                "export default async function activate(plugin) {{\n\
                 const store = plugin.storage.json(\"choice\", {{ defaults: {{ on: false }}, debounceMs: 0 }});\n\
                 let view = null;\n\
                 const apply = () => {{\n\
                 if (!store.value.on) return;\n\
                 (view ??= plugin.video.view()).set({{ zoom: 1.5 }});\n\
                 console.error(`{TAKES_UP} applied`);\n\
                 }};\n\
                 plugin.commands.add({{ id: \"on\", path: [\"Tools\", \"Zoom\"],\n\
                 run: () => {{ store.save({{ on: true }}); apply(); }} }});\n\
                 apply();\n}}\n"
            ),
        )]
    });
    let lazy = [("VESTRA_IDLE_MS", "500")];

    // First run: nothing live, parked once quiet with its command recorded.
    let first = run(root.path(), 5, &lazy);
    assert!(
        first.contains(&format!("lazy: {TAKES_UP} classified lazy")),
        "{first}"
    );
    assert!(!first.contains(&format!("{TAKES_UP} applied")), "{first}");
    assert!(
        first.contains(&format!("lazy: {TAKES_UP} parked")),
        "{first}"
    );

    // Replayed, woken by its command, which takes up the video view. The
    // default idle time (30 s): the player quits before any idle standby
    // asks the worker again, as the user who turns shaders on and quits.
    let woken = [("VESTRA_TEST_EXECUTE", "vestra.fixture-takes-up/on@2500")];
    let second = run(root.path(), 6, &woken);
    assert!(
        second.contains(&format!("lazy: {TAKES_UP} replayed (1 calls)")),
        "{second}"
    );
    assert!(second.contains(&format!("{TAKES_UP} applied")), "{second}");
    assert!(
        second.contains(&format!("lazy: {TAKES_UP} resident from now on")),
        "{second}"
    );
    assert!(
        recordings(root.path())
            .iter()
            .any(|r| r["residentAtReady"] == true),
        "a resident recording: {:?}",
        recordings(root.path())
    );

    // Launched at start, not replayed: its activation applies its choice.
    let third = run(root.path(), 4, &lazy);
    assert!(
        third.contains(&format!(
            "lazy: {TAKES_UP} starts: resident at its last ready"
        )),
        "{third}"
    );
    assert!(
        !third.contains(&format!("lazy: {TAKES_UP} replayed")),
        "{third}"
    );
    assert!(third.contains(&format!("{TAKES_UP} applied")), "{third}");
}

const SETS_AFTER_READY: &str = "vestra.fixture-sets-after-ready";

/// A first run (an install, or any upgrade) whose activation returns before
/// it takes up what its saved settings ask for: the request reaches the
/// worker after the ready, while the classification's question is out (held
/// out 300 ms by `VESTRA_TEST_CLASSIFY_LATE`, so the request always lands in
/// that window), and the answer, asked before the request, says nothing is
/// held. The question is asked again; the run is classified resident, not
/// parked with a lazy recording, and the next start launches it.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_first_run_that_takes_up_a_resource_while_its_class_is_asked_is_not_parked() {
    let root = root_with(|plugins| {
        vec![package(
            plugins,
            SETS_AFTER_READY,
            &["commands", "processing.video"],
            format!(
                "export default async function activate(plugin) {{\n\
                 plugin.commands.add({{ id: \"noop\", path: [\"Tools\", \"Noop\"], run: () => {{}} }});\n\
                 setTimeout(() => {{\n\
                 plugin.video.view().set({{ zoom: 1.5 }});\n\
                 console.error(`{SETS_AFTER_READY} applied`);\n\
                 }}, 0);\n}}\n"
            ),
        )]
    });
    let first = run(root.path(), 4, &[("VESTRA_TEST_CLASSIFY_LATE", "300")]);
    assert!(
        first.contains(&format!("{SETS_AFTER_READY} applied")),
        "{first}"
    );
    let applied = first.find(&format!("{SETS_AFTER_READY} applied")).unwrap();
    let handled = first
        .find("VESTRA_TEST_CLASSIFY_LATE")
        .unwrap_or_else(|| panic!("{first}"));
    assert!(applied < handled, "the request came while asked: {first}");
    assert!(
        first.contains(&format!("lazy: {SETS_AFTER_READY} classified resident")),
        "{first}"
    );
    assert!(
        !first.contains(&format!("lazy: {SETS_AFTER_READY} parked")),
        "{first}"
    );
    assert!(
        recordings(root.path())
            .iter()
            .any(|r| r["residentAtReady"] == true),
        "a resident recording: {:?}\n{first}",
        recordings(root.path())
    );
    let second = run(root.path(), 3, &[]);
    assert!(
        second.contains(&format!(
            "lazy: {SETS_AFTER_READY} starts: resident at its last ready"
        )),
        "{second}"
    );
    assert!(
        second.contains(&format!("{SETS_AFTER_READY} applied")),
        "{second}"
    );
}

const RESTORES_AFTER_ROUND_TRIP: &str = "vestra.fixture-restores-after-round-trip";
const RESTORES_AFTER_TIMER: &str = "vestra.fixture-restores-after-timer";

/// A first run (an install, or any upgrade) whose activation starts the
/// restore of its saved effect and returns (`void apply()`): one applies
/// it after a round trip to the host and a `setTimeout(0)`, one after a
/// 300 ms timer (a slower read), each in a player of its own (the video
/// view has one owner). Neither waits for a test hook: the 300 ms request
/// always comes after the class was answered lazy. A first run parked at
/// its classification stopped before such a request came and dropped it,
/// and its lazy recording did not restore the effect at the next start.
/// Now a first run goes to standby only after a quiet period: the request
/// reaches the worker while it runs, makes it resident, and the next start
/// launches it.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_first_run_that_restores_its_effect_after_its_ready_starts_at_the_next_launch() {
    for (id, wait) in [
        (
            RESTORES_AFTER_ROUND_TRIP,
            "await plugin.storage.session.get(\"saved\");\n\
             await new Promise((done) => setTimeout(done, 0));",
        ),
        (
            RESTORES_AFTER_TIMER,
            "await new Promise((done) => setTimeout(done, 300));",
        ),
    ] {
        let root = root_with(|plugins| {
            vec![package(
                plugins,
                id,
                &["commands", "processing.video"],
                format!(
                    "export default async function activate(plugin) {{\n\
                     plugin.commands.add({{ id: \"noop\", path: [\"Tools\", \"Noop\"], run: () => {{}} }});\n\
                     const apply = async () => {{\n\
                     {wait}\n\
                     await plugin.video.view().set({{ zoom: 1.5 }});\n\
                     console.error(`{id} applied`);\n\
                     }};\n\
                     void apply();\n}}\n"
                ),
            )]
        });
        let first = run(root.path(), 4, &[]);
        assert!(first.contains(&format!("{id} applied")), "{id}: {first}");
        assert!(
            !first.contains(&format!("lazy: {id} parked")),
            "{id}: {first}"
        );
        // Classified resident (the request came while its class was asked)
        // or resident from the request on: either way not lazy.
        assert!(
            first.contains(&format!("lazy: {id} classified resident"))
                || first.contains(&format!("lazy: {id} resident from now on")),
            "{id}: {first}"
        );
        assert!(
            recordings(root.path())
                .iter()
                .any(|r| r["residentAtReady"] == true),
            "{id}: a resident recording: {:?}\n{first}",
            recordings(root.path())
        );
        let second = run(root.path(), 3, &[]);
        assert!(
            second.contains(&format!("lazy: {id} starts: resident at its last ready")),
            "{id}: {second}"
        );
        assert!(
            !second.contains(&format!("lazy: {id} replayed")),
            "{id}: {second}"
        );
        assert!(second.contains(&format!("{id} applied")), "{id}: {second}");
    }
}

/// The old lazy activation pinned at most eight first runs in flight
/// (`FIRST_RUNS`), built-ins
/// included. Now built-ins start outside every limit (supervisor.md §5
/// "Start order and limits"): built-ins' first runs are outside
/// the start-job limit and start at once; third-party ones (below,
/// `third_party_first_runs_start_a_few_at_a_time`) at most `max_active` at a
/// time, a place freed at each ready.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn b3_built_in_first_runs_start_at_once() {
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
        &[("VESTRA_SHARED", "1"), ("VESTRA_IDLE_MS", "60000")],
    );
    let starts = log
        .match_indices("starts for its first run")
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    assert_eq!(starts.len(), 10, "{log}");
    let first_park = log
        .find("parked (1 recorded calls)")
        .unwrap_or_else(|| panic!("{log}"));
    assert!(starts[9] < first_park, "{log}");
    for id in &ids {
        let parked = format!("lazy: {id} parked (1 recorded calls)");
        assert!(log.contains(&parked), "{parked}: {log}");
    }
    assert_eq!(recordings(root.path()).len(), 10);
}

/// doc/architecture/supervisor.md §5: ten installed third-party
/// plugins on a fresh profile start their first runs at most `max_active`
/// (`min(4, max(2, cores/2))`) at a time: a place frees at each one's
/// `runtime.ready`. All ten run, none times out, all are parked.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn third_party_first_runs_start_a_few_at_a_time() {
    let root = root_with(|_| Vec::new());
    let store = root.path().join("store");
    let mut catalog =
        serde_json::json!({"installed":{},"pending":{},"operations":{},"lastCommit":{}});
    let ids = (0..10)
        .map(|n| format!("aa.fixture-third-{n}"))
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
        12,
        &[("VESTRA_SHARED", "1"), ("VESTRA_IDLE_MS", "60000")],
    );
    let max = log
        .lines()
        .find_map(|line| {
            let rest = line.split(" starts at once").next()?;
            rest.rsplit(' ').next()?.parse::<usize>().ok()
        })
        .unwrap_or_else(|| panic!("no start limit logged: {log}"));
    assert!((2..=4).contains(&max), "{max}");
    let (mut in_flight, mut most, mut starts) = (0_usize, 0_usize, 0_usize);
    for line in log.lines() {
        if line.contains("starts for its first run") {
            in_flight += 1;
            starts += 1;
            most = most.max(in_flight);
        } else if line.contains("runtime.ready") {
            in_flight = in_flight.saturating_sub(1);
        }
    }
    assert_eq!(starts, 10, "{log}");
    assert_eq!(most, max, "at most {max} at once: {log}");
    for id in &ids {
        let parked = format!("lazy: {id} parked (1 recorded calls)");
        assert!(log.contains(&parked), "{parked}: {log}");
    }
    assert!(!log.contains("timed out"), "{log}");
    assert_eq!(recordings(root.path()).len(), 10);
}

/// Installs `id` in the library under `store` as a package from a file:
/// its files in `store/packages/<digest>`, its record in `catalog.json`
/// (`installed`, the rest left empty). `n` makes its instance GUID.
fn install(store: &Path, catalog: &mut serde_json::Value, n: usize, id: &str, main: String) {
    let manifest = serde_json::json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1",
        "entry":"main.js"});
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

/// First runs of other plugins never take the places while a
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
        &[("VESTRA_SHARED", "1"), ("VESTRA_IDLE_MS", "60000")],
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

/// A command whose `enabled` is a `plugin.when` condition is
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
    let lazy = [("VESTRA_SHARED", "1"), ("VESTRA_IDLE_MS", "500")];
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

/// A listener of `packages.changed` (a wake event) does not keep
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
    let lazy = [("VESTRA_SHARED", "1"), ("VESTRA_IDLE_MS", "500")];
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

const BINDINGS: &str = "vestra.fixture-bindings";
const LATE: &str = "vestra.fixture-late";

/// Wake events are muted during start-up (nothing wakes), and a muted one
/// is not dropped for a plugin parked before it (V2, SDK 1.9.0: the hotkeys
/// page parked with 337 rows instead of 372): once the mute ends, a plugin
/// parked in this run whose event's state is not the one it last had is
/// woken once, and reads the current state. A plugin registering a command
/// late in start-up (its activation takes 4 s) changes `bindings.list`
/// after the listener parked (idle 500 ms). The next start, with the same
/// plugins and commands, wakes nothing.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_listener_parked_at_start_up_is_woken_once_for_a_muted_change() {
    let root = root_with(|plugins| {
        vec![
            package(
                plugins,
                BINDINGS,
                &["commands"],
                format!(
                    "export default async function activate(plugin) {{\n\
                     const late = async () => (await plugin.bindings.list()).commands\n\
                     .filter((c) => c.plugin === \"{LATE}\").length;\n\
                     console.error(`{BINDINGS} start sees ${{await late()}}`);\n\
                     plugin.bindings.onChange(async () =>\n\
                     console.error(`{BINDINGS} heard ${{await late()}}`));\n}}\n"
                ),
            ),
            package(
                plugins,
                LATE,
                &["commands"],
                format!(
                    "export default async function activate(plugin) {{\n\
                     await new Promise((done) => setTimeout(done, 4000));\n\
                     plugin.commands.add({{ id: \"late\", path: [\"Tools\", \"Late\"], run: () => {{}} }});\n\
                     console.error(`{LATE} added`);\n}}\n"
                ),
            ),
        ]
    });
    let lazy = [("VESTRA_SHARED", "1"), ("VESTRA_IDLE_MS", "500")];
    let first = run(root.path(), 10, &lazy);
    let at = |text: &str| {
        first
            .find(text)
            .unwrap_or_else(|| panic!("{text}: {first}"))
    };
    // The set-up: the listener parked before the late command came, during
    // start-up.
    let parked = at(&format!("lazy: {BINDINGS} parked"));
    let added = at(&format!("{LATE} added"));
    let settled = at("lazy: start-up settled");
    assert!(parked < added && added < settled, "{first}");
    // Not woken while muted; once the mute ended, woken once for it.
    let woken = at(&format!("lazy: {BINDINGS} woken by bindings.changed"));
    assert!(settled < woken, "{first}");
    assert_eq!(
        count(&first, &format!("lazy: {BINDINGS} woken by")),
        1,
        "{first}"
    );
    assert!(
        first.contains(&format!(
            "lazy: {BINDINGS} parked before bindings.changed changed at start-up, waking it"
        )),
        "{first}"
    );
    // Its woken run sees the late command.
    assert!(
        first[woken..].contains(&format!("{BINDINGS} start sees 1"))
            || first[woken..].contains(&format!("{BINDINGS} heard 1")),
        "{first}"
    );

    // The next start: both replayed, the same commands; nothing is woken.
    let second = run(root.path(), 4, &lazy);
    for id in [BINDINGS, LATE] {
        assert!(second.contains(&format!("lazy: {id} replayed")), "{second}");
        assert!(!second.contains(&format!("lazy: {id} woken")), "{second}");
        assert!(
            !second.contains(&format!("lazy: {id} parked before")),
            "{second}"
        );
    }
    assert!(!second.contains(&format!("] {BINDINGS} start")), "{second}");
}

/// A test parks and wakes every built-in and diffs the registries: for
/// each built-in, what the host's registries
/// hold for it (commands with declaration, shown text, bar button, state
/// and conditions in registration order; key bindings; settings pages;
/// drop providers; wake events: `VESTRA_TEST_REGISTRY`, host/supervisor/probe.rs)
/// after a normal activation must equal what they hold after record → park,
/// after replay at the next start, and after its wake. `VESTRA_TEST_PARK=all`
/// parks resident built-ins too (their live resources are not recorded, so
/// only the registries are compared); `VESTRA_TEST_WAKE` wakes every
/// parked one. A difference names the plugin, the step and the entry.
/// Steps: a normal activation's ready → the first run's ready (`parking`)
/// → `parked` → `replayed` at the next start → `woken` (its ready after the
/// prune), and a normal run at quit → the woken run at quit. Commands'
/// states and pages' values are compared exactly from the park to the
/// replay and at quit; at the two readies only the registrations are, since
/// plugins set some of those values after their ready. Recorded marks: all
/// entries while parked or replayed, none once running.
///
/// The real built-ins: `VESTRA_TEST_BUILTINS=<dir with builtin.json>` (a
/// staged `resources/plugins`), else staged here with
/// `node scripts/stage-plugins.mjs` (needs `npm ci` in `src/`). Built-ins
/// whose activation never completes headless (a panel mount waits for a
/// window) cannot be parked and are listed, not failed.
#[test]
#[ignore = "needs Node 26, libmpv and the staged built-ins (npm ci in src): run with --include-ignored (ci:local)"]
fn every_built_in_keeps_its_registry_through_park_replay_and_wake() {
    let root = tempfile::tempdir().unwrap();
    let plugins = match std::env::var_os("VESTRA_TEST_BUILTINS") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => {
            let staged = root.path().join("plugins");
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
            let status = Command::new("node")
                .arg("scripts/stage-plugins.mjs")
                .arg(&staged)
                .current_dir(&source)
                .status()
                .expect("node on PATH");
            assert!(status.success(), "stage-plugins.mjs failed");
            staged
        }
    };
    let builtins = serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(plugins.join("builtin.json")).expect("builtin.json"),
    )
    .unwrap()["plugins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let probe = |name: &str, seconds: u32, env: &[(&str, &str)]| {
        let file = root.path().join(format!("{name}.jsonl"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
        command
            .args(["--headless", "--silent", "--seconds", &seconds.to_string()])
            .env("VESTRA_BUILTIN_PLUGINS", &plugins)
            .env("VESTRA_PACKAGE_STORE", root.path().join("store"))
            .env("VESTRA_PLUGIN_DATA", root.path().join("data"))
            .env("APPDATA", root.path().join("appdata"))
            .env("LOCALAPPDATA", root.path().join("local"))
            .env("VESTRA_TEST_REGISTRY", &file);
        for key in [
            "VESTRA_SHARED",
            "VESTRA_IDLE_MS",
            "VESTRA_TEST_EXECUTE",
            "VESTRA_TEST_PARK",
            "VESTRA_TEST_WAKE",
            "VESTRA_INSTANCE_DIR",
        ] {
            command.env_remove(key);
        }
        for (key, value) in env {
            command.env(key, value);
        }
        let output = command.output().unwrap();
        let log = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(output.status.success(), "{name}: {log}");
        let lines = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        (lines, log)
    };
    // The first snapshot at `moment` of `id` (the last one at `quit`).
    let at = |lines: &[serde_json::Value], moment: &str, id: &str| {
        let mut found = lines
            .iter()
            .filter(|line| line["moment"] == moment && line["id"] == id);
        let line = if moment == "quit" {
            found.next_back()
        } else {
            found.next()
        };
        line.map(|line| line["registry"].clone())
    };

    // A: every built-in started at launch (its first run).
    let (normal, _) = probe("normal", 12, &[]);
    // B: first runs, each parked at its ready.
    let park = [("VESTRA_TEST_PARK", "all"), ("VESTRA_IDLE_MS", "600000")];
    let (parks, parks_log) = probe("park", 15, &park);
    // C: replayed at start, all woken at 4 s, still running at quit.
    let mut wake = park.to_vec();
    wake.push(("VESTRA_TEST_WAKE", "4000"));
    let (wakes, wakes_log) = probe("wake", 15, &wake);

    let mut problems = Vec::new();
    let mut covered = Vec::new();
    let mut uncovered = Vec::new();
    for id in &builtins {
        // At its ready (a park of a first run happens then) and at quit
        // (what it changed after its ready, asynchronously, is there).
        let (Some(ready), Some(live)) = (at(&normal, "ready", id), at(&normal, "quit", id)) else {
            uncovered.push(format!(
                "{id}: its activation never completes headless (no ready with lazy activation off)"
            ));
            continue;
        };
        let steps = [
            ("parking", &parks, "parking"),
            ("parked", &parks, "parked"),
            ("replayed", &wakes, "replayed"),
            ("woken", &wakes, "woken"),
        ];
        let mut before = ("a normal activation (ready)", ready);
        for (label, lines, moment) in steps {
            let Some(now) = at(lines, moment, id) else {
                problems.push(format!("{id}: no `{moment}` snapshot ({label})"));
                break;
            };
            let parked = matches!(moment, "parked" | "replayed");
            // Between two activations (a normal one and a first run; the
            // recording and a wake) only the registrations are compared: a
            // command's state and a page's values are what the plugin sets
            // around its ready, some of it after (video's render-mode reason,
            // hotkeys' list), compared exactly from the park to the replay
            // and at quit.
            let (mut a, mut b) = (before.1.clone(), now.clone());
            if matches!(moment, "parking" | "woken") {
                for registry in [&mut a, &mut b] {
                    for (section, field) in [("commands", "state"), ("pages", "properties")] {
                        for entry in registry[section].as_array_mut().into_iter().flatten() {
                            entry[field] = serde_json::Value::Null;
                        }
                    }
                }
            }
            problems.extend(
                registry_diff(&a, &b)
                    .into_iter()
                    .map(|d| format!("{id}: {} → {label}: {d}", before.0)),
            );
            problems.extend(
                recorded_marks(&now, parked)
                    .into_iter()
                    .map(|d| format!("{id}: {label}: {d}")),
            );
            before = (label, now);
        }
        // Running after its wake against running normally, both at quit.
        match at(&wakes, "quit", id) {
            Some(end) => {
                let label = "running after its wake (quit)";
                problems.extend(
                    registry_diff(&live, &end)
                        .into_iter()
                        .map(|d| format!("{id}: a normal activation (quit) → {label}: {d}")),
                );
                problems.extend(
                    recorded_marks(&end, false)
                        .into_iter()
                        .map(|d| format!("{id}: {label}: {d}")),
                );
            }
            None => problems.push(format!("{id}: not running after its wake (quit)")),
        }
        let count = |section: &str| live[section].as_array().map_or(0, Vec::len);
        covered.push(format!(
            "{id}: {} commands, {} bindings, {} pages, {} drop providers, wake events {}",
            count("commands"),
            count("bindings"),
            count("pages"),
            count("drops"),
            live["wakes"]
        ));
    }
    // Shown with --nocapture (`eprintln!` is disallowed: it panics on a
    // closed stderr).
    let _ = writeln!(
        std::io::stderr(),
        "registry diff: covered\n  {}\nnot covered\n  {}",
        covered.join("\n  "),
        uncovered.join("\n  ")
    );
    assert!(
        problems.is_empty(),
        "{} difference(s):\n{}\n--- park run\n{parks_log}\n--- wake run\n{wakes_log}",
        problems.len(),
        problems.join("\n")
    );
    // Only the built-ins with a panel mount (refused headless, so their
    // activation fails) may be left out: one that stops reaching its ready
    // fails here instead of shrinking the coverage unnoticed. Audio,
    // subtitles and video mount their adjust panels since the four built-in
    // panels (menu redesign step 4). Capture mounts nothing since its tab went
    // (snapshots only; clip export is a store plugin).
    const MOUNTS: [&str; 6] = [
        "vestra.audio",
        "vestra.playback",
        "vestra.playlist",
        "vestra.subtitles",
        "vestra.video",
        "vestra.visualizer",
    ];
    let unexpected = uncovered
        .iter()
        .filter(|line| !MOUNTS.iter().any(|id| line.starts_with(&format!("{id}:"))))
        .collect::<Vec<_>>();
    assert!(
        unexpected.is_empty(),
        "built-ins not covered: {unexpected:?}"
    );
    assert!(
        // 16 built-ins less the 6 that mount a panel (10 since capture's
        // tab went).
        covered.len() >= builtins.len().saturating_sub(MOUNTS.len()) && covered.len() >= 10,
        "{} of {} built-ins covered: {uncovered:?}",
        covered.len(),
        builtins.len()
    );
}

/// The differences between two registry snapshots of one plugin, the
/// `recorded` marks left out: entries by id per section (missing, added,
/// moved, changed fields) and the wake events.
fn registry_diff(a: &serde_json::Value, b: &serde_json::Value) -> Vec<String> {
    type Entries = Vec<(String, serde_json::Value)>;
    let mut out = Vec::new();
    for section in ["commands", "bindings", "pages", "drops"] {
        let list = |v: &serde_json::Value| -> Entries {
            v[section]
                .as_array()
                .into_iter()
                .flatten()
                .map(|entry| {
                    let mut entry = entry.clone();
                    if let Some(object) = entry.as_object_mut() {
                        object.remove("recorded");
                    }
                    (entry["id"].as_str().unwrap_or_default().to_owned(), entry)
                })
                .collect()
        };
        let (before, after) = (list(a), list(b));
        let has = |entries: &Entries, id: &str| entries.iter().any(|(each, _)| each == id);
        for (id, _) in before.iter().filter(|(id, _)| !has(&after, id)) {
            out.push(format!("{section} `{id}` missing"));
        }
        for (id, _) in after.iter().filter(|(id, _)| !has(&before, id)) {
            out.push(format!("{section} `{id}` added"));
        }
        let order = |of: &Entries, other: &Entries| {
            of.iter()
                .filter(|(id, _)| has(other, id))
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        };
        let (order_a, order_b) = (order(&before, &after), order(&after, &before));
        if order_a != order_b {
            out.push(format!(
                "{section} in another order: {order_a:?} → {order_b:?}"
            ));
        }
        for (id, entry) in &before {
            if let Some((_, other)) = after.iter().find(|(each, _)| each == id) {
                leaf_diff(&format!("{section} `{id}`"), entry, other, &mut out);
            }
        }
    }
    if a["wakes"] != b["wakes"] {
        out.push(format!("wake events: {} → {}", a["wakes"], b["wakes"]));
    }
    out
}

/// The differing leaves of `a` and `b` (objects by key, lists of one
/// length by index), each as `<at>.<path>: <a> → <b>`.
fn leaf_diff(at: &str, a: &serde_json::Value, b: &serde_json::Value, out: &mut Vec<String>) {
    use serde_json::Value;
    if a == b {
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys().filter(|k| !x.contains_key(*k))) {
                let (left, right) = (x.get(key), y.get(key));
                leaf_diff(
                    &format!("{at}.{key}"),
                    left.unwrap_or(&Value::Null),
                    right.unwrap_or(&Value::Null),
                    out,
                );
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (n, (left, right)) in x.iter().zip(y).enumerate() {
                leaf_diff(&format!("{at}[{n}]"), left, right, out);
            }
        }
        _ => out.push(format!("{at}: {a} → {b}")),
    }
}

/// Entries whose recorded mark is not `expected` (parked or replayed: all
/// recorded; running: none).
fn recorded_marks(registry: &serde_json::Value, expected: bool) -> Vec<String> {
    let mut out = Vec::new();
    for section in ["commands", "pages", "drops"] {
        for entry in registry[section].as_array().into_iter().flatten() {
            if entry["recorded"] != expected {
                out.push(format!(
                    "{section} `{}` recorded: {}, expected {expected}",
                    entry["id"].as_str().unwrap_or_default(),
                    entry["recorded"]
                ));
            }
        }
    }
    out
}
