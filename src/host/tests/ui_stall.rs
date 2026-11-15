//! The UI stall test (doc/architecture/core-thread.md §8, core thread plan
//! S1.2): the windowed host runs offscreen (`VESTRA_TEST_OFFSCREEN=1`) and
//! `VESTRA_TEST_UI_STALL` holds its UI thread once, as Slint compiling a
//! window, Windows' move/size loop or a GPU wait do. Meanwhile:
//!
//! - a development plugin asks `source.state` every 50 ms and logs each
//!   request's latency;
//! - a built-in whose first run goes quiet just before the stall is due to
//!   go to standby (park) during it (a lazy first run parks after 1 s
//!   without use, supervisor.md §3);
//! - a development plugin has a `time` trigger due during the stall.
//!
//! The same three are measured outside the stall as a control: polls after
//! start-up settled and before the stall, and from 1 s after it (the
//! backlog drained); a second built-in parked and a second trigger due
//! well before the stall.
//!
//! Rules R1 and R7 say the stall must cost none of them. On today's code
//! the core runs on the UI thread, so all three are late by about the
//! stall's length: the three stall assertions **fail**, and that is
//! asserted (`STALL_ASSERTIONS_HOLD`, expected to change in S3a), so the
//! test proves the instrument sees the stall and keeps `ci:local` green
//! until the core has its own thread. The control assertions hold today
//! and must keep holding. The measured numbers are printed (and written
//! to `$VESTRA_TEST_LOG_DIR/ui_stall.json` when set): they are the
//! baseline.
//!
//! Needs Node 26 on `PATH` (or `VESTRA_NODE`), the development runner and
//! libmpv. Ignored by default; `npm run ci:local` runs it
//! (`--include-ignored`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// **Expected to change in S3a** (core thread plan): `false` while the
/// core runs on the UI thread, so the stall assertions are expected to
/// fail; S3a sets it to `true`, and from then on they must hold.
const STALL_ASSERTIONS_HOLD: bool = false;

/// When the stall begins, after the player process started, and how long
/// it lasts (`VESTRA_TEST_UI_STALL`).
const STALL_AT_MS: u64 = 12_000;
const STALL_MS: u64 = 4_000;
/// The run's length: the stall, then time for the backlog and the control
/// polls after it.
const RUN_SECONDS: u64 = (STALL_AT_MS + STALL_MS) / 1000 + 5;

/// The thresholds: the park on time; the trigger within one core turn
/// (16 ms) plus its way to the plugin's handler (pipe, Node), on a debug
/// build. Request latency p99: the design (core-thread.md §8) says 50 ms,
/// but today's 16 ms cadence already gives 66 ms without a stall (a
/// `source.state` request waits for a turn, the worker's reply for
/// another; measured 2026-11-15, debug build, p99 66 ms over 133 and 68
/// polls before and after the stall), and S3a keeps that cadence (rule
/// R10). So 100 ms, one and a half times that floor and far below a stall's
/// seconds; S3b (event-driven) may lower it.
const LATENCY_P99_MS: u64 = 100;
const PARK_LATE_MS: i64 = 1_000;
const TRIGGER_LATE_MS: i64 = 100;
/// A lazy first run's quiet time before standby (supervisor.md §3).
const FIRST_RUN_QUIET_MS: i64 = 1_000;

const POLLER: &str = "vestra.fixture-stall-poll";
const TRIGGERS: &str = "vestra.fixture-stall-trigger";
const PARK_CONTROL: &str = "vestra.fixture-stall-park-control";
const PARK_STALL: &str = "vestra.fixture-stall-park-stall";

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

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

/// A plugin package `id` with `main` under `root`; its `builtin.json` entry.
fn package(root: &Path, id: &str, main: String) -> Value {
    let files = [
        (
            "manifest.json",
            json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1","entry":"main.js"})
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
    json!({"id":id,"version":"1.0.0","directory":id,"digest":digest(&files)})
}

/// Asks `source.state` every 50 ms: `ui-stall poll <sent> <took ms>`.
fn poller() -> String {
    "export default function activate(plugin) {\n\
       setInterval(() => {\n\
         const sent = Date.now();\n\
         plugin.player.refresh().then(\n\
           () => console.error(`ui-stall poll ${sent} ${Date.now() - sent}`),\n\
           (error) => console.error(`ui-stall poll-error ${sent} ${error}`));\n\
       }, 50);\n\
     }\n"
    .to_owned()
}

/// Two `time` triggers: `ui-stall trigger <name> <due> <fired>`.
fn triggers(control_due: i64, stall_due: i64) -> String {
    format!(
        "export default function activate(plugin) {{\n\
           for (const [name, due] of [[\"control\", {control_due}], [\"stall\", {stall_due}]])\n\
             plugin.trigger(name)\n\
               .firesOn(plugin.when.time({{ min: due }}))\n\
               .enter(() => console.error(`ui-stall trigger ${{name}} ${{due}} ${{Date.now()}}`));\n\
         }}\n"
    )
}

/// A built-in that is used (a request every 100 ms) until `quiet_at`,
/// then not: `ui-stall park <name> quiet <at>`, and at its stop
/// `ui-stall park <name> stop <at> <reason>`.
fn parker(name: &str, quiet_at: i64) -> String {
    format!(
        "export default function activate(plugin) {{\n\
           plugin.onDispose((info) =>\n\
             console.error(`ui-stall park {name} stop ${{Date.now()}} ${{info?.reason}}`));\n\
           void (async () => {{\n\
             while (Date.now() < {quiet_at}) {{\n\
               await plugin.storage.session.get(\"k\");\n\
               await new Promise((done) => setTimeout(done, 100));\n\
             }}\n\
             console.error(`ui-stall park {name} quiet ${{Date.now()}}`);\n\
           }})();\n\
         }}\n"
    )
}

/// One stderr line as it arrived (epoch ms).
struct Line {
    at: i64,
    text: String,
}

/// Run the player offscreen with the stall; its stderr lines, stamped.
fn run(root: &Path, development: &[PathBuf]) -> Vec<Line> {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    command
        .args(["--silent", "--seconds", &RUN_SECONDS.to_string()])
        .env("VESTRA_TEST_OFFSCREEN", "1")
        .env("VESTRA_TEST_UI_STALL", format!("{STALL_AT_MS},{STALL_MS}"))
        .env("VESTRA_BUILTIN_PLUGINS", root.join("plugins"))
        .env("VESTRA_PACKAGE_STORE", root.join("store"))
        .env("VESTRA_PLUGIN_DATA", root.join("data"))
        .env("VESTRA_UI_STATE", root.join("ui-state.json"))
        .env("VESTRA_INSTANCE_DIR", root.join("instance"))
        .env("VESTRA_SHADER_CACHE", root.join("shader-cache"))
        .env("APPDATA", root.join("appdata"))
        .env("LOCALAPPDATA", root.join("local"))
        .env("VESTRA_LANGUAGE", "en");
    for key in [
        "VESTRA_SHARED",
        "VESTRA_IDLE_MS",
        "VESTRA_TEST_EXECUTE",
        "VESTRA_TEST_PARK",
        "VESTRA_TEST_WAKE",
        "VESTRA_TEST_WIRE_TRACE",
        "VESTRA_LOG_FILE",
        "VESTRA_FRAME_LOG",
    ] {
        command.env_remove(key);
    }
    for directory in development {
        command.arg("--plugin").arg(directory);
    }
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
            .map(|text| Line {
                at: epoch_ms(),
                text,
            })
            .collect::<Vec<_>>()
    });
    let status = child.wait().unwrap();
    let lines = reader.join().unwrap();
    let log = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(status.success(), "{log}");
    lines
}

/// The words after `ui-stall ` in a plugin's line (`[<id>] ` may precede).
fn fields(text: &str) -> Option<Vec<&str>> {
    let at = text.find("ui-stall ")?;
    Some(text[at + "ui-stall ".len()..].split_whitespace().collect())
}

/// The 99th percentile (nearest rank) of `values`, `None` when empty.
fn p99(values: &[i64]) -> Option<i64> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let rank = (sorted.len() * 99).div_ceil(100);
    rank.checked_sub(1).map(|index| sorted[index])
}

#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_stalled_ui_thread_holds_requests_parks_and_triggers_today() {
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    let development = root.path().join("development");
    std::fs::create_dir_all(&plugins).unwrap();
    std::fs::create_dir_all(&development).unwrap();
    // The player starts within a few ms of this; the stall begins
    // `STALL_AT_MS` after its process started (or at its first tick after
    // that, on a slow start: checked below).
    let t0 = epoch_ms();
    let stall_at = t0 + STALL_AT_MS as i64;
    let entries = vec![
        package(&plugins, PARK_CONTROL, parker("control", t0 + 6_000)),
        // Quiet 400 ms before the stall: due to park 600 ms into it.
        package(&plugins, PARK_STALL, parker("stall", stall_at - 400)),
    ];
    std::fs::write(
        plugins.join("builtin.json"),
        json!({"schema":3,"plugins":entries}).to_string(),
    )
    .unwrap();
    package(&development, POLLER, poller());
    package(
        &development,
        TRIGGERS,
        triggers(t0 + 9_000, stall_at + 1_500),
    );
    let lines = run(
        root.path(),
        &[development.join(POLLER), development.join(TRIGGERS)],
    );
    let log = lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let stamp = |text: &str| {
        lines
            .iter()
            .find(|line| line.text.contains(text))
            .map(|line| line.at)
            .unwrap_or_else(|| panic!("no `{text}` line:\n{log}"))
    };
    let began = stamp("test: UI stall of ");
    let ended = stamp("test: UI stall ended");
    let settled = stamp("lazy: start-up settled");
    let end = lines.last().map_or(ended, |line| line.at);

    let mut polls = Vec::new();
    let mut poll_errors = Vec::new();
    let mut parks =
        std::collections::BTreeMap::<String, (Option<i64>, Option<(i64, String)>)>::new();
    let mut fired = std::collections::BTreeMap::<String, (i64, i64)>::new();
    for line in &lines {
        let Some(words) = fields(&line.text) else {
            continue;
        };
        let number = |index: usize| words.get(index).and_then(|w| w.parse::<i64>().ok());
        match words.first().copied() {
            Some("poll") => polls.push((number(1).unwrap(), number(2).unwrap())),
            // A request on its way when the player quit (as V2 drops it).
            Some("poll-error") if !line.text.contains("HostError: plugin stop") => {
                poll_errors.push(line.text.clone())
            }
            Some("park") => {
                let entry = parks.entry(words[1].to_owned()).or_default();
                match words[2] {
                    "quiet" => entry.0 = number(3),
                    "stop" => entry.1 = Some((number(3).unwrap(), words[4].to_owned())),
                    _ => {}
                }
            }
            Some("trigger") => {
                fired.insert(
                    words[1].to_owned(),
                    (number(2).unwrap(), number(3).unwrap()),
                );
            }
            _ => {}
        }
    }
    assert!(poll_errors.is_empty(), "{poll_errors:?}\n{log}");

    // The setup itself: the stall came when asked and lasted, and what is
    // due during it falls inside it.
    assert!(
        (began - stall_at).abs() < 1_000,
        "the stall began {} ms from when it was asked (a slow start?)\n{log}",
        began - stall_at
    );
    assert!(
        ended - began >= STALL_MS as i64 - 50,
        "the stall lasted {} ms\n{log}",
        ended - began
    );
    assert!(settled + 1_000 < t0 + 6_000, "start-up settled late\n{log}");

    let latencies = |from: i64, to: i64| {
        polls
            .iter()
            .filter(|(sent, _)| (from..to).contains(sent))
            .map(|(_, took)| *took)
            .collect::<Vec<_>>()
    };
    let before = latencies(settled + 1_000, began);
    let during = latencies(began, ended);
    let after = latencies(ended + 1_000, end - 500);
    for (name, values) in [("before", &before), ("during", &during), ("after", &after)] {
        assert!(
            values.len() >= 10,
            "{} polls {name} the stall\n{log}",
            values.len()
        );
    }

    let park = |name: &str| {
        let (quiet, stop) = parks
            .get(name)
            .cloned()
            .unwrap_or_else(|| panic!("park {name}: nothing logged\n{log}"));
        let quiet = quiet.unwrap_or_else(|| panic!("park {name}: never quiet\n{log}"));
        let (stopped, reason) = stop.unwrap_or_else(|| panic!("park {name}: never parked\n{log}"));
        assert_eq!(reason, "idle", "park {name} is a standby\n{log}");
        let due = quiet + FIRST_RUN_QUIET_MS;
        (due, stopped - due)
    };
    let (control_park_due, control_park_late) = park("control");
    let (stall_park_due, stall_park_late) = park("stall");
    assert!(
        (began..ended - 500).contains(&stall_park_due),
        "the park is due during the stall\n{log}"
    );
    let trigger = |name: &str| {
        let (due, at) = *fired
            .get(name)
            .unwrap_or_else(|| panic!("trigger {name}: never fired\n{log}"));
        (due, at - due)
    };
    let (_, control_trigger_late) = trigger("control");
    let (stall_trigger_due, stall_trigger_late) = trigger("stall");
    assert!(
        (began..ended - 500).contains(&stall_trigger_due),
        "the trigger is due during the stall\n{log}"
    );
    assert!(control_park_due < began && control_trigger_late > -1_000);

    let report = json!({
        "stall": {"asked_ms": STALL_AT_MS, "length_ms": STALL_MS,
            "began_after_asked_ms": began - stall_at, "lasted_ms": ended - began},
        "latency_ms": {
            "before": {"count": before.len(), "p99": p99(&before), "max": before.iter().max()},
            "during": {"count": during.len(), "p99": p99(&during), "max": during.iter().max()},
            "after": {"count": after.len(), "p99": p99(&after), "max": after.iter().max()},
        },
        "park_late_ms": {"control": control_park_late, "stall": stall_park_late},
        "trigger_late_ms": {"control": control_trigger_late, "stall": stall_trigger_late},
        "stall_assertions_hold": STALL_ASSERTIONS_HOLD,
    });
    let _ = writeln!(std::io::stderr(), "ui stall baseline: {report:#}");
    if let Some(dir) = std::env::var_os("VESTRA_TEST_LOG_DIR") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ui_stall.json"), format!("{report:#}\n")).unwrap();
        std::fs::write(dir.join("ui_stall.log"), &log).unwrap();
    }

    // Outside the stall: these hold today and must keep holding.
    for (name, values) in [("before", &before), ("after", &after)] {
        let p = p99(values).unwrap();
        assert!(
            p < LATENCY_P99_MS as i64,
            "latency p99 {name} the stall: {p} ms\n{report:#}"
        );
    }
    assert!(
        control_park_late < PARK_LATE_MS,
        "the control park was {control_park_late} ms late\n{report:#}"
    );
    assert!(
        control_trigger_late < TRIGGER_LATE_MS,
        "the control trigger was {control_trigger_late} ms late\n{report:#}"
    );

    // During the stall (rules R1, R7): expected to fail until S3a.
    let stall = [
        (
            "request latency p99 during the stall",
            p99(&during).unwrap() < LATENCY_P99_MS as i64,
        ),
        ("the park on time", stall_park_late < PARK_LATE_MS),
        (
            "the trigger within one core turn",
            stall_trigger_late < TRIGGER_LATE_MS,
        ),
    ];
    for (what, holds) in stall {
        assert_eq!(
            holds,
            STALL_ASSERTIONS_HOLD,
            "{what}: {} (expected {} on this code; STALL_ASSERTIONS_HOLD)\n{report:#}",
            if holds { "holds" } else { "fails" },
            if STALL_ASSERTIONS_HOLD {
                "to hold"
            } else {
                "to fail"
            },
        );
    }
}

#[test]
fn p99_is_the_nearest_rank() {
    assert_eq!(p99(&[]), None);
    assert_eq!(p99(&[7]), Some(7));
    let hundred: Vec<i64> = (1..=100).collect();
    assert_eq!(p99(&hundred), Some(99));
    let two_hundred: Vec<i64> = (1..=200).collect();
    assert_eq!(p99(&two_hundred), Some(198));
}
