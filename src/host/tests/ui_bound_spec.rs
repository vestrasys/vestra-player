//! Spec tests of the core thread's handoff rules, written from the design
//! (doc/architecture/core-thread.md §5, rules R5, R6, R12 and R13; core
//! thread plan S2d: "spec tests written from rules R5, R6, R12 and R13
//! pass") by a separate author, without reading the implementation. They
//! pin what a plugin can observe, so they hold on today's code (the core
//! on the UI thread), after S2d (the explicit seam, one thread) and after
//! S3a (two threads).
//!
//! Each test runs the windowed host offscreen (`VESTRA_TEST_OFFSCREEN=1`)
//! with the wire trace (`VESTRA_TEST_WIRE_TRACE`) and controlled plugins.
//! The plugins send their requests by name (`plugin.wire.request`), so the
//! wire shows exactly the calls the rules speak of, and log what each
//! answered as `SPEC|<tag>|<json>`. The assertions read those lines and the
//! plugin's own lines in the trace. Lines written to one plugin are
//! compared by their trace `seq` only with each other: they are written in
//! the core's order by one writer, while the order of a line read against
//! a line written is the pipes' timing. Every trace must also pass
//! `src/scripts/verify-wire.mjs check` (pairing, R4's UI-bound order, R5,
//! R13 as one trace shows them).
//!
//! What the offscreen platform cannot show: it has no native window, so
//! `window.setVideoArea` and `window.setPictureInPicture` are refused at
//! once; the four read-after-await cases are checked for `ui.mount` and
//! `layout.set` (cases 1 and 4) in full, and for the two window methods
//! (cases 2 and 3) only as "whatever the reply said, a read right after it
//! agrees with it". The 1 s timeout path of `window.setVideoArea` cannot
//! be reached offscreen.
//!
//! Needs Node 26 on `PATH` (or `VESTRA_NODE`), the development runner and
//! libmpv. Ignored by default; `npm run ci:local` runs them
//! (`--include-ignored`). The one that can hold only on two threads is
//! ignored with "after S3a".
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
mod isolation;

fn epoch_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

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

/// What every test plugin's `main.js` starts with: `log(tag, value)`,
/// `outcome(promise)` (`{ok}` or `{error: code}`) and `pause(ms)`.
const PRELUDE: &str = "const log = (tag, value) => console.error(`SPEC|${tag}|${JSON.stringify(value ?? null)}`);\n\
     const outcome = (promise) => promise.then(\n\
       (result) => ({ ok: result ?? null }),\n\
       (error) => ({ error: error?.code ?? String(error) }));\n\
     const pause = (ms) => new Promise((done) => setTimeout(done, ms));\n";

/// A sidebar panel (`ui.mount`).
const PANEL: &str = "export global Bridge {\n    in property <bool> tabbed;\n}\n\
     export component List inherits Rectangle {\n    in property <string> label;\n\
     background: #1a1821;\n    Text { text: label; color: white; }\n}\n";

/// A tool window (`ui.open`).
const WINDOW: &str = "export component Spec inherits Window {\n\
     in property <string> label;\n    width: 200px;\n    height: 100px;\n\
     Text { text: label; }\n}\n";

/// A tool window that does not compile.
const BROKEN: &str = "export component Broken inherits Window {\n\
     Text { text: no-such-property; }\n}\n";

/// The UI files every package carries.
const UI: [(&str, &str); 3] = [
    ("ui/list.slint", PANEL),
    ("ui/win.slint", WINDOW),
    ("ui/broken.slint", BROKEN),
];

/// A plugin package `id` under `root` with `body` after the prelude; its
/// `builtin.json` entry.
fn package(root: &Path, id: &str, body: &str) -> Value {
    let mut files = vec![
        (
            "manifest.json",
            json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1","entry":"main.js"})
                .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", format!("{PRELUDE}{body}")),
    ];
    files.extend(UI.iter().map(|(name, text)| (*name, (*text).to_owned())));
    let directory = root.join(id);
    for (name, text) in &files {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    json!({"id":id,"version":"1.0.0","directory":id,"digest":digest(&files)})
}

/// One offscreen run: its stderr and its wire trace.
struct Run {
    log: String,
    trace: Vec<Value>,
    trace_path: PathBuf,
}

impl Run {
    /// The values `plugin` logged under `tag`, in order.
    fn spec(&self, tag: &str) -> Vec<Value> {
        let marker = format!("SPEC|{tag}|");
        self.log
            .lines()
            .filter_map(|line| {
                let at = line.find(&marker)?;
                serde_json::from_str(&line[at + marker.len()..]).ok()
            })
            .collect()
    }
    /// The one value logged under `tag`.
    fn one(&self, tag: &str) -> Value {
        let values = self.spec(tag);
        assert_eq!(values.len(), 1, "one `{tag}` line:\n{}", self.log);
        values.into_iter().next().unwrap()
    }
    /// `plugin`'s trace records (lines only), in `seq` order.
    fn records(&self, plugin: &str) -> Vec<&Value> {
        let mut records = self
            .trace
            .iter()
            .filter(|r| r["plugin"] == plugin && r.get("line").is_some())
            .collect::<Vec<_>>();
        records.sort_by_key(|r| r["seq"].as_u64());
        records
    }
}

/// Runs the host offscreen for `seconds` under `root` with the built-ins
/// `builtins`, the development plugins `development` and `env`.
fn run(
    root: &Path,
    builtins: &[Value],
    development: &[PathBuf],
    seconds: u32,
    env: &[(&str, String)],
) -> Run {
    let directory = root.join("builtins");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("builtin.json"),
        json!({"schema":3,"plugins":builtins}).to_string(),
    )
    .unwrap();
    let trace_path = root.join("run.wire.jsonl");
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    isolation::isolate(&mut command, root);
    command
        .args(["--silent", "--seconds", &seconds.to_string()])
        .env("VESTRA_TEST_OFFSCREEN", "1")
        .env("VESTRA_TEST_WIRE_TRACE", &trace_path)
        .env("VESTRA_BUILTIN_PLUGINS", &directory)
        .env("VESTRA_INSTANCE_DIR", root.join("instance"))
        .env("VESTRA_LANGUAGE", "en");
    for key in [
        "VESTRA_SHARED",
        "VESTRA_IDLE_MS",
        "VESTRA_TEST_EXECUTE",
        "VESTRA_TEST_PARK",
        "VESTRA_TEST_WAKE",
        "VESTRA_TEST_UI_STALL",
        "VESTRA_FRAME_LOG",
    ] {
        command.env_remove(key);
    }
    for (key, value) in env {
        command.env(key, value);
    }
    for plugin in development {
        command.arg("--plugin").arg(plugin);
    }
    let output = command.output().unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    let trace = std::fs::read_to_string(&trace_path)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    if let Some(dir) = std::env::var_os("VESTRA_TEST_LOG_DIR") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let name = root.file_name().unwrap();
        let _ = std::fs::copy(&trace_path, dir.join(name).with_extension("wire.jsonl"));
        let _ = std::fs::write(dir.join(name).with_extension("log"), &log);
    }
    let run = Run {
        log,
        trace,
        trace_path,
    };
    check_trace(&run);
    run
}

/// `node src/scripts/verify-wire.mjs check <trace>` passes.
fn check_trace(run: &Run) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/verify-wire.mjs");
    let node = std::env::var_os("VESTRA_NODE").unwrap_or_else(|| "node".into());
    let output = Command::new(node)
        .arg(script)
        .arg("check")
        .arg(&run.trace_path)
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.status.code(), Some(0), "verify-wire check:\n{text}");
}

/// A development plugin `id` with `body` under `root/development`.
fn development(root: &Path, id: &str, body: &str) -> PathBuf {
    let directory = root.join("development");
    package(&directory, id, body);
    directory.join(id)
}

fn request<'a>(records: &[&'a Value], method: &str, matches: impl Fn(&Value) -> bool) -> &'a Value {
    let found = records
        .iter()
        .filter(|r| r["dir"] == "in" && r["line"]["method"] == method)
        .filter(|r| matches(&r["line"]["params"]))
        .collect::<Vec<_>>();
    assert_eq!(found.len(), 1, "one {method} request matching");
    found[0]
}

/// The reply to `request` (same process, same id).
fn reply_to<'a>(records: &[&'a Value], request: &Value) -> &'a Value {
    records
        .iter()
        .find(|r| {
            r["dir"] == "out"
                && r["owner"] == request["owner"]
                && r["line"]["id"] == request["line"]["id"]
                && r["line"].get("method").is_none()
                && (r["line"].get("result").is_some() || r["line"].get("error").is_some())
        })
        .unwrap_or_else(|| panic!("a reply to {}", request["line"]))
}

fn seq(record: &Value) -> u64 {
    record["seq"].as_u64().unwrap()
}

/// The events named `event` written to the plugin's process `owner`.
fn events<'a>(records: &[&'a Value], owner: &Value, event: &str) -> Vec<&'a Value> {
    records
        .iter()
        .copied()
        .filter(|r| r["dir"] == "out" && r["owner"] == *owner && r["line"]["event"] == event)
        .collect()
}

const VIEW: &str = "test.spec-view";

/// R5: after `ui.open`'s reply the view exists. A `ui.set` sent after the
/// awaited reply finds it, a second `ui.open` of the same id is a conflict,
/// and after `ui.close`'s reply it is gone. The UI-bound stream keeps its
/// execution order (R4): `ui.open`, `ui.set`, `ui.show`, `ui.close` and
/// another `ui.set` sent without awaiting are carried out in that order.
/// A window that does not compile: the reply carries the error (an
/// interface result), and no view exists after it.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r5_the_ui_open_reply_means_the_view_exists() {
    let root = tempfile::tempdir().unwrap();
    let plugin = development(
        root.path(),
        VIEW,
        "export default function activate(plugin) {\n\
           const w = plugin.wire;\n\
           const open = (id, file) => outcome(w.request(\"ui.open\", { id, file, properties: { label: id }, callbacks: [] }));\n\
           const set = (id, value) => outcome(w.request(\"ui.set\", { id, property: \"label\", value }));\n\
           setTimeout(async () => {\n\
             log(\"open\", await open(\"a\", \"ui/win.slint\"));\n\
             log(\"set-after-open\", await set(\"a\", \"x\"));\n\
             log(\"open-again\", await open(\"a\", \"ui/win.slint\"));\n\
             log(\"show\", await outcome(w.request(\"ui.show\", { id: \"a\", visible: true })));\n\
             log(\"close\", await outcome(w.request(\"ui.close\", { id: \"a\" })));\n\
             log(\"set-after-close\", await set(\"a\", \"y\"));\n\
             log(\"unawaited\", await Promise.all([\n\
               open(\"b\", \"ui/win.slint\"),\n\
               set(\"b\", \"z\"),\n\
               outcome(w.request(\"ui.show\", { id: \"b\", visible: false })),\n\
               outcome(w.request(\"ui.close\", { id: \"b\" })),\n\
               set(\"b\", \"after close\"),\n\
             ]));\n\
             log(\"open-broken\", await open(\"c\", \"ui/broken.slint\"));\n\
             log(\"set-broken\", await set(\"c\", \"v\"));\n\
             log(\"done\", true);\n\
           }, 1000);\n\
         }\n",
    );
    let run = run(root.path(), &[], &[plugin], 8, &[]);
    assert_eq!(run.one("done"), json!(true), "{}", run.log);
    let ok = json!({"ok": null});
    assert_eq!(run.one("open"), ok, "ui.open answered without an error");
    assert_eq!(
        run.one("set-after-open"),
        ok,
        "the view exists after the reply"
    );
    assert_eq!(run.one("open-again"), json!({"error":"conflict"}));
    assert_eq!(run.one("show"), ok);
    assert_eq!(run.one("close"), ok);
    assert_eq!(
        run.one("set-after-close"),
        json!({"error":"missing"}),
        "the view is gone after ui.close's reply"
    );
    assert_eq!(
        run.one("unawaited"),
        json!([ok, ok, ok, ok, {"error":"missing"}]),
        "the UI-bound stream carries out a plugin's requests in arrival order"
    );
    let broken = run.one("open-broken");
    assert!(
        broken.get("error").is_some(),
        "a compile error is the reply: {broken}"
    );
    assert_eq!(
        run.one("set-broken"),
        json!({"error":"missing"}),
        "no view after a failed open"
    );
}

const MOUNT: &str = "test.spec-mount";

/// R5, R6 and R13 for `ui.mount` and `layout.set`, after start-up settled:
///
/// - R5: `ui.preparing` (`preparing`, then `ready`) is written before the
///   `ui.mount` reply; after the reply the mount is ready (`ui.setMount`
///   finds it). A mount of a component that does not compile: `failed`,
///   then the error reply.
/// - R13 case 1: `layout.state` sent right after the `ui.mount` reply lists
///   the panel in the sidebar.
/// - R13 case 4: `layout.state` sent right after the `layout.set` reply
///   shows the change (the sidebar hidden, then shown again).
/// - R13 order, for `ui.mount`: no `window.state` broadcast goes out
///   between the request and its reply (nothing else changes meanwhile);
///   the one that shows the change comes after the reply. Not for
///   `layout.set`: today its `window.state` precedes its reply (see below).
/// - R13 cases 2 and 3, as far as offscreen goes: whatever
///   `window.setVideoArea` and `window.setPictureInPicture` answered, the
///   `window.state` read right after agrees with it (the new size or
///   state when accepted; the unchanged state when refused).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r5_r6_r13_a_mount_and_a_layout_change_are_in_place_when_answered() {
    let root = tempfile::tempdir().unwrap();
    let plugin = development(
        root.path(),
        MOUNT,
        "export default function activate(plugin) {\n\
           const w = plugin.wire;\n\
           const req = (method, params) => outcome(w.request(method, params));\n\
           plugin.player.subscribe();\n\
           w.on(\"window.state\", () => {});\n\
           setTimeout(async () => {\n\
             log(\"layout-before\", await req(\"layout.state\"));\n\
             log(\"mount\", await req(\"ui.mount\", { slot: \"sidebar\", file: \"ui/list.slint\",\n\
               component: \"List\", global: \"Bridge\", title: \"Spec\", properties: { tabbed: true } }));\n\
             log(\"layout-after-mount\", await req(\"layout.state\"));\n\
             log(\"set-mount\", await req(\"ui.setMount\", { property: \"tabbed\", value: false }));\n\
             await pause(800);\n\
             log(\"hide\", await req(\"layout.set\", { slot: \"sidebar\", visible: false }));\n\
             log(\"layout-after-hide\", await req(\"layout.state\"));\n\
             await pause(800);\n\
             log(\"show\", await req(\"layout.set\", { slot: \"sidebar\", visible: true }));\n\
             log(\"layout-after-show\", await req(\"layout.state\"));\n\
             await pause(800);\n\
             log(\"window-before\", await req(\"window.state\"));\n\
             log(\"area\", await req(\"window.setVideoArea\", { width: 640, height: 360 }));\n\
             log(\"window-after-area\", await req(\"window.state\"));\n\
             await pause(800);\n\
             log(\"pip-before\", await req(\"window.state\"));\n\
             log(\"pip\", await req(\"window.setPictureInPicture\", { on: true }));\n\
             log(\"window-after-pip\", await req(\"window.state\"));\n\
             await pause(500);\n\
             log(\"broken\", await req(\"ui.mount\", { slot: \"bottom\", file: \"ui/broken.slint\",\n\
               component: \"Broken\", properties: {} }));\n\
             log(\"done\", true);\n\
           }, 2000);\n\
         }\n",
    );
    let run = run(root.path(), &[], &[plugin], 12, &[]);
    assert_eq!(run.one("done"), json!(true), "{}", run.log);
    let ok = |tag: &str| {
        let value = run.one(tag);
        assert!(value.get("ok").is_some(), "{tag}: {value}");
        value["ok"].clone()
    };
    // R5: the mount is ready after its reply.
    ok("mount");
    ok("set-mount");
    // R13 case 1: the read right after the reply sees the mount.
    let before = ok("layout-before");
    let panels = |layout: &Value| {
        layout["sidebar"]["panels"]
            .as_array()
            .map(|panels| panels.iter().filter(|p| p["plugin"] == MOUNT).count())
            .unwrap_or(0)
    };
    assert_eq!(panels(&before), 0, "no panel before: {before}");
    let mounted = ok("layout-after-mount");
    assert_eq!(panels(&mounted), 1, "the panel is listed: {mounted}");
    assert_eq!(mounted["sidebar"]["preferred"], true, "{mounted}");
    // R13 case 4: each layout.set is in place when answered.
    ok("hide");
    let hidden = ok("layout-after-hide");
    assert_eq!(hidden["sidebar"]["preferred"], false, "{hidden}");
    assert_eq!(hidden["sidebar"]["effective"], false, "{hidden}");
    ok("show");
    let shown = ok("layout-after-show");
    assert_eq!(shown["sidebar"]["preferred"], true, "{shown}");
    assert_eq!(shown["sidebar"]["effective"], true, "{shown}");
    // R13 cases 2 and 3: a read right after the reply agrees with it.
    let window_before = ok("window-before");
    let area = run.one("area");
    let after_area = ok("window-after-area");
    match area.get("ok") {
        Some(size) => assert_eq!(
            (
                after_area["videoArea"]["width"].as_f64(),
                after_area["videoArea"]["height"].as_f64()
            ),
            (size["width"].as_f64(), size["height"].as_f64()),
            "the read after the setVideoArea reply shows its size: {after_area}"
        ),
        None => assert_eq!(
            after_area["videoArea"], window_before["videoArea"],
            "refused ({area}): the read shows the size before"
        ),
    }
    let pip_before = ok("pip-before");
    let pip = run.one("pip");
    let after_pip = ok("window-after-pip");
    match pip.get("ok") {
        Some(_) => assert_eq!(
            after_pip["pictureInPicture"]["active"], true,
            "the read after the setPictureInPicture reply shows it: {after_pip}"
        ),
        None => assert_eq!(
            after_pip["pictureInPicture"], pip_before["pictureInPicture"],
            "refused ({pip}): the read shows the state before"
        ),
    }
    let broken = run.one("broken");
    assert!(broken.get("error").is_some(), "{broken}");

    // The wire.
    let records = run.records(MOUNT);
    let mount = request(&records, "ui.mount", |p| p["slot"] == "sidebar");
    let mount_reply = reply_to(&records, mount);
    let owner = &mount["owner"];
    let preparing = events(&records, owner, "ui.preparing")
        .into_iter()
        .filter(|e| e["line"]["data"]["slot"] == "sidebar")
        .collect::<Vec<_>>();
    let phases = preparing
        .iter()
        .map(|e| e["line"]["data"]["phase"].as_str().unwrap_or_default())
        .collect::<Vec<_>>();
    assert_eq!(phases, ["preparing", "ready"], "R5: {preparing:?}");
    assert!(
        preparing.iter().all(|e| seq(e) < seq(mount_reply)),
        "R5, R6: ui.preparing before the ui.mount reply"
    );
    let failed = request(&records, "ui.mount", |p| p["slot"] == "bottom");
    let failed_reply = reply_to(&records, failed);
    assert!(failed_reply["line"].get("error").is_some());
    let failed_phases = events(&records, owner, "ui.preparing")
        .into_iter()
        .filter(|e| e["line"]["data"]["slot"] == "bottom")
        .map(|e| (e["line"]["data"]["phase"].clone(), seq(e)))
        .collect::<Vec<_>>();
    // Refused before it reached the interface (a component that does not
    // compile is `invalid` at once today), or failed there: then its
    // `ui.preparing` ends `failed`.
    if !failed_phases.is_empty() {
        assert_eq!(
            failed_phases.last().map(|p| p.0.clone()),
            Some(json!("failed")),
            "{failed_phases:?}"
        );
    }
    assert!(
        failed_phases.iter().all(|p| p.1 < seq(failed_reply)),
        "R5, R6: ui.preparing failed before the error reply"
    );
    // R13 order: no window.state between a request and its reply; the
    // first one after the request comes after the reply. For `ui.mount`
    // only: today `layout.set`'s `window.state` (showing the change) is
    // written *before* its reply (seen 2026-11-15 on core-s2c7-rest), and
    // S2 must not change the wire, so the design's "reply, then
    // broadcast" is not asserted for it (reported to the coordinator).
    let states = events(&records, owner, "window.state");
    let between = states
        .iter()
        .filter(|e| seq(e) > seq(mount) && seq(e) < seq(mount_reply))
        .count();
    assert_eq!(
        between, 0,
        "R13: no window.state between ui.mount and its reply"
    );
    assert!(
        states.iter().any(|e| seq(e) > seq(mount_reply)),
        "a window.state follows the mount (the sidebar took its width)"
    );
    // R13's `layout.set` exception, as on main: its `window.state`, showing
    // the change, is written before its reply (S3a(3) re-review B1: the core
    // sends a facts report's broadcast before the next report that writes a
    // line, here the reply).
    for visible in [false, true] {
        let set = request(&records, "layout.set", |params| {
            params["visible"] == visible
        });
        let set_reply = reply_to(&records, set);
        assert!(
            states.iter().any(|e| seq(e) > seq(set)
                && seq(e) < seq(set_reply)
                && e["line"]["data"]["slots"]["sidebar"]["effective"] == visible),
            "R13: layout.set (visible {visible}): its window.state before its reply"
        );
    }
}

const ORDER: &str = "test.spec-order";

/// R6: events and broadcasts keep the core's order.
///
/// - A side effect of a request goes out before its reply: a trigger whose
///   condition already holds when it is set (`phase: empty`) fires, and its
///   `triggers.fired` is written before the `triggers.set` reply.
/// - Triggers that flip are delivered before the state broadcast that
///   flipped them: opening a file that does not exist takes the source to
///   `loading` and then `error`; for each phase the plugin's trigger on it
///   fires (its `triggers.fired` carries the state with that phase) before
///   the first `source.state` broadcast showing that phase.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r6_side_effects_before_the_reply_and_triggers_before_their_broadcast() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("no-such-clip.mkv");
    let plugin = development(
        root.path(),
        ORDER,
        &format!(
            "export default function activate(plugin) {{\n\
               plugin.player.subscribe();\n\
               plugin.player.onChange(() => {{}});\n\
               for (const phase of [\"loading\", \"error\"])\n\
                 plugin.trigger(`on-${{phase}}`).firesOn(plugin.when.phase(phase))\n\
                   .enter(() => log(`fired-${{phase}}`, true));\n\
               plugin.trigger(\"already\").firesOn(plugin.when.phase(\"ended\"))\n\
                 .enter(() => log(\"fired-already\", true));\n\
               setTimeout(async () => {{\n\
                 log(\"set-already\", await outcome(plugin.wire.request(\"triggers.set\",\n\
                   {{ trigger: \"already\", when: {{ phase: [\"empty\"] }} }})));\n\
               }}, 1500);\n\
               setTimeout(() => {{\n\
                 plugin.player.open({missing}).then(\n\
                   (media) => media.ready()).then(\n\
                   () => log(\"opened\", true), (error) => log(\"open-failed\", error?.code ?? String(error)));\n\
               }}, 3000);\n\
             }}\n",
            missing = json!(missing.to_string_lossy())
        ),
    );
    let run = run(root.path(), &[], &[plugin], 9, &[]);
    let set_already = run.one("set-already");
    assert!(set_already.get("ok").is_some(), "{set_already}");
    let records = run.records(ORDER);
    let fired = |name: &str| {
        records
            .iter()
            .copied()
            .filter(|r| r["dir"] == "out" && r["line"]["event"] == "triggers.fired")
            .filter(|r| r["line"]["data"]["trigger"].to_string().contains(name))
            .filter(|r| r["line"]["data"]["edge"] == "enter")
            .collect::<Vec<_>>()
    };
    // A side effect before the reply.
    let set = request(&records, "triggers.set", |p| {
        p["trigger"] == "already" && p["when"]["phase"] == json!(["empty"])
    });
    let set_reply = reply_to(&records, set);
    let already = fired("already");
    assert_eq!(already.len(), 1, "fired once: {already:?}");
    assert!(
        seq(already[0]) < seq(set_reply),
        "R6: triggers.fired (seq {}) before the triggers.set reply (seq {})",
        seq(already[0]),
        seq(set_reply)
    );
    // Triggers before the broadcast that flipped them.
    let states = records
        .iter()
        .copied()
        .filter(|r| r["dir"] == "out" && r["line"]["event"] == "source.state")
        .collect::<Vec<_>>();
    let mut checked = Vec::new();
    for phase in ["loading", "error"] {
        let Some(broadcast) = states.iter().find(|r| r["line"]["data"]["phase"] == phase) else {
            continue;
        };
        let flipped = fired(&format!("on-{phase}"));
        assert!(
            !flipped.is_empty(),
            "the trigger on {phase} fired: {}",
            run.log
        );
        assert_eq!(flipped[0]["line"]["data"]["state"]["phase"], phase);
        assert!(
            seq(flipped[0]) < seq(broadcast),
            "R6: the {phase} trigger (seq {}) before the source.state that flipped it (seq {})",
            seq(flipped[0]),
            seq(broadcast)
        );
        checked.push(phase);
    }
    assert!(
        checked.contains(&"error"),
        "the failed open reached error: {}",
        run.log
    );
}

const WOKEN_MOUNT: &str = "vestra.fixture-spec-woken-mount";
const WOKEN_OPEN: &str = "vestra.fixture-spec-woken-open";

/// A built-in whose activation logs its start and sets a `time` trigger due
/// at `due` (epoch ms). Its first run does nothing else, so it goes quiet
/// and is parked (a lazy first run, supervisor.md §3). The run its trigger
/// wakes sends `ui` (an initial interface request, not awaited, so that
/// `runtime.ready` comes before its reply), and logs when it was answered.
fn woken(due: i64, ui: &str) -> String {
    format!(
        "export default async function activate(plugin) {{\n\
           log(\"start\", Date.now());\n\
           plugin.trigger(\"due\").firesOn(plugin.when.time({{ min: {due} }}))\n\
             .enter(() => log(\"fired\", Date.now()));\n\
           const n = ((await plugin.storage.session.get(\"starts\")) ?? 0) + 1;\n\
           await plugin.storage.session.set(\"starts\", n);\n\
           if (n > 1) plugin.wire.request({ui}).then(\n\
             () => log(\"answered\", Date.now()), (error) => log(\"refused\", error?.code ?? String(error)));\n\
         }}\n"
    )
}

/// The woken-run scenario (R12): two built-ins, parked after their first
/// run; their `time` trigger wakes them at `due_after` ms after the start;
/// each woken run sends its initial interface request (`ui.mount` of a
/// sidebar panel, `ui.open` of a window) and gets the held trigger only
/// once ready. With `stall` (`VESTRA_TEST_UI_STALL`: ms after the player
/// started, length), the wake falls inside the stall. Returns the run and
/// the epoch ms just before the player started.
fn woken_run(due_after: i64, seconds: u32, stall: Option<(u64, u64)>) -> (Run, i64) {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let builtins = root.path().join("builtins");
    let due = started + due_after;
    let entries = [
        package(
            &builtins,
            WOKEN_MOUNT,
            &woken(
                due,
                "\"ui.mount\", { slot: \"sidebar\", file: \"ui/list.slint\", component: \"List\", global: \"Bridge\", title: \"Woken\", properties: {} }",
            ),
        ),
        package(
            &builtins,
            WOKEN_OPEN,
            &woken(
                due,
                "\"ui.open\", { id: \"woken\", file: \"ui/win.slint\", properties: {}, callbacks: [] }",
            ),
        ),
    ];
    let mut env = Vec::new();
    if let Some((at, length)) = stall {
        env.push(("VESTRA_TEST_UI_STALL", format!("{at},{length}")));
    }
    let run = run(root.path(), &entries, &[], seconds, &env);
    for id in [WOKEN_MOUNT, WOKEN_OPEN] {
        assert!(
            run.log.contains(&format!("lazy: {id} parked")),
            "{id}'s first run is parked: {}",
            run.log
        );
        assert_eq!(
            run.plugin_spec(id, "start").len(),
            2,
            "{id} started, then woken: {}",
            run.log
        );
        assert_eq!(
            run.plugin_spec(id, "answered").len(),
            1,
            "{id}'s interface request answered: {}",
            run.log
        );
        assert_eq!(
            run.plugin_spec(id, "fired").len(),
            1,
            "{id}'s held trigger: {}",
            run.log
        );
    }
    (run, started)
}

impl Run {
    /// The numbers `plugin` logged under `tag` (its lines carry `[<id>]`).
    fn plugin_spec(&self, plugin: &str, tag: &str) -> Vec<i64> {
        let marker = format!("[{plugin}] SPEC|{tag}|");
        self.log
            .lines()
            .filter_map(|line| {
                let at = line.find(&marker)?;
                line[at + marker.len()..].trim().parse().ok()
            })
            .collect()
    }
}

/// R12 on one thread: a run is ready only after its first `ui.open` /
/// `ui.mount` was answered, also when its `runtime.ready` came first. A
/// run woken by its trigger holds the trigger until it is ready, so the
/// woken run's `triggers.fired` is written after the reply to its
/// `ui.mount` (or `ui.open`), and that reply after its `runtime.ready`
/// was read.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r12_a_woken_run_gets_its_trigger_only_after_its_first_ui_request_was_answered() {
    let (run, _) = woken_run(6_000, 10, None);
    for (plugin, method) in [(WOKEN_MOUNT, "ui.mount"), (WOKEN_OPEN, "ui.open")] {
        let records = run.records(plugin);
        let asked = request(&records, method, |_| true);
        let answered = reply_to(&records, asked);
        assert!(
            answered["line"].get("result").is_some(),
            "{plugin}: {answered}"
        );
        let fired = events(&records, &asked["owner"], "triggers.fired");
        assert_eq!(fired.len(), 1, "{plugin}: {}", run.log);
        assert!(
            seq(fired[0]) > seq(answered),
            "R12: {plugin}'s held trigger (seq {}) after its {method} reply (seq {})",
            seq(fired[0]),
            seq(answered)
        );
        // For `ui.mount` (answered after the mount batch, so later than
        // the plugin's `runtime.ready`): the run was not ready at its
        // runtime.ready, the case R12 is about. `ui.open` is answered in
        // the same turn on one thread, so its order against runtime.ready
        // is the pipes' timing there.
        if method != "ui.mount" {
            continue;
        }
        let ready = records
            .iter()
            .find(|r| {
                r["dir"] == "in"
                    && r["owner"] == asked["owner"]
                    && r["line"]["event"] == "runtime.ready"
            })
            .unwrap_or_else(|| panic!("{plugin}'s woken run is ready: {}", run.log));
        assert!(
            ready["ms"].as_u64() <= answered["ms"].as_u64(),
            "{plugin}: the scenario needs runtime.ready before the {method} reply"
        );
    }
}

/// When the stall begins (ms after the player started) and how long it
/// lasts; the woken runs' trigger is due inside it.
const STALL_AT_MS: u64 = 6_000;
const STALL_MS: u64 = 4_000;

/// R12 across a stall (design R12: "a run that starts during an interface
/// stall therefore becomes ready when the stall ends"; plan §4): the
/// trigger wakes the parked built-ins during a UI stall, so their
/// processes start while the UI is held (the core runs on); their initial
/// `ui.mount` / `ui.open` wait for the UI, so the runs become ready, and
/// get their held trigger, only after the stall ended.
///
/// Only possible with the core on its own thread: on one thread the wake
/// itself waits for the stall to end.
#[test]
#[ignore = "after S3a: on one thread the wake waits for the stall (the core runs on the UI thread); needs Node 26 and libmpv"]
fn r12_a_run_that_starts_during_a_ui_stall_is_ready_after_it() {
    let (run, started) = woken_run(
        STALL_AT_MS as i64 + 1_000,
        ((STALL_AT_MS + STALL_MS) / 1000 + 5) as u32,
        Some((STALL_AT_MS, STALL_MS)),
    );
    // The stall is timed from the player's start, a little after `started`.
    let stall_begins = started + STALL_AT_MS as i64;
    let stall_ends = stall_begins + STALL_MS as i64;
    for id in [WOKEN_MOUNT, WOKEN_OPEN] {
        let woken_start = run.plugin_spec(id, "start")[1];
        assert!(
            woken_start < stall_ends - 1_000,
            "{id}: the woken run started during the stall ({} ms after it began): {}",
            woken_start - stall_begins,
            run.log
        );
        let answered = run.plugin_spec(id, "answered")[0];
        let fired = run.plugin_spec(id, "fired")[0];
        assert!(
            answered >= stall_ends && fired >= answered,
            "{id}: ready after the stall: answered {} ms and fired {} ms after it ended",
            answered - stall_ends,
            fired - stall_ends
        );
    }
}
