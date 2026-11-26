//! Spec tests of the core thread, step S3a (core thread plan §3 "S3a
//! brief", the core thread design's rules R4, R7, R8, R12, R13, K8 and
//! K9), written from the design by a separate author
//! without reading the implementation. They pin what a plugin can observe
//! while the interface (the UI thread) is stalled.
//!
//! Each test runs the windowed host offscreen (`VESTRA_TEST_OFFSCREEN=1`)
//! with the wire trace (`VESTRA_TEST_WIRE_TRACE`), controlled plugins and,
//! where named, one UI stall (`VESTRA_TEST_UI_STALL=<at ms>,<length ms>`,
//! timed from the player's start). The plugins send their requests by name
//! (`plugin.wire.request`) and log what each answered as `SPEC|<tag>|<json>`;
//! the assertions read those lines and the plugins' lines in the trace.
//! Lines written to one plugin are compared by their trace `seq` only with
//! each other (one writer, the core). Every trace must also pass
//! `src/scripts/verify-wire.mjs check`.
//!
//! Two kinds of test:
//!
//! - Tests that hold on today's code (the core on the UI thread) and must
//!   keep holding on two threads: ignored by default like every offscreen
//!   test ("needs Node 26 and libmpv"), run by `ci:local`
//!   (`--include-ignored`).
//! - Tests that can hold only once the core has its own thread: ignored
//!   with a reason that starts "after S3a". On today's code they fail at the
//!   assertion named in their comment. **Flip point:** S3a's merge removes
//!   the "after S3a" from their `#[ignore]` (keeping the Node/libmpv reason),
//!   together with `STALL_ASSERTIONS_HOLD = true` in `ui_stall.rs` and the
//!   R12 stall case in `ui_bound_spec.rs`. Until then `ci:local` shows them
//!   failing, as it shows that R12 case. (Flipped in plan step S3a(4), the
//!   thread; they hold only with the core on its own thread.)
//!
//! Not testable through today's public harness (each says which hook it
//! needs, in its own comment): R8 needs a UI-side key injection hook; groups A/G need displays
//! offscreen (`app.system` lists none there today); K9
//! assumes the run's end (`--seconds`) is a quit the core acts on itself.
//!
//! Needs Node 26 on `PATH` (or `VESTRA_NODE`), the development runner and
//! libmpv.
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
mod isolation;

/// When the stall begins (ms after the player started) and how long it
/// lasts, as in `ui_bound_spec.rs`.
const STALL_AT_MS: i64 = 6_000;
const STALL_MS: i64 = 4_000;
/// What "did not wait for the UI" means here: answered within this, where
/// a wait costs the stall (4 s). The core period is 16 ms (R10), a
/// worker-forwarded request takes two turns; far below a second.
const PROMPT_MS: i64 = 1_000;

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
/// `outcome(promise)` (`{ok}` or `{error: code}`), `pause(ms)`,
/// `until(epoch ms)`, and `req(plugin, method, params)` (a request by name;
/// a request the SDK refuses before sending it gives `{thrown: code}`).
const PRELUDE: &str = r#"const log = (tag, value) => console.error(`SPEC|${tag}|${JSON.stringify(value ?? null)}`);
const outcome = (promise) => promise.then(
  (result) => ({ ok: result ?? null }),
  (error) => ({ error: error?.code ?? String(error) }));
const pause = (ms) => new Promise((done) => setTimeout(done, ms));
const until = (epoch) => pause(Math.max(0, epoch - Date.now()));
const req = (plugin, method, params) => {
  try { return outcome(plugin.wire.request(method, params)); }
  catch (error) { return Promise.resolve({ thrown: error?.code ?? String(error) }); }
};
"#;

/// A sidebar or bottom panel (`ui.mount`).
const PANEL: &str = "export global Bridge {\n    in property <bool> tabbed;\n}\n\
     export component List inherits Rectangle {\n    in property <string> label;\n\
     background: #1a1821;\n    Text { text: label; color: white; }\n}\n";

/// A tool window (`ui.open`).
const WINDOW: &str = "export component Spec inherits Window {\n\
     in property <string> label;\n    width: 200px;\n    height: 100px;\n\
     Text { text: label; }\n}\n";

/// A panel or settings page with a list property in its bridge global.
const ROWS: &str = "export global Rows {\n    in property <[{id: string}]> rows;\n}\n\
     export component RowsPanel inherits Rectangle {\n    background: #1a1821;\n\
     VerticalLayout { for row in Rows.rows: Text { text: row.id; color: white; } }\n}\n";

/// A tool window with a list property.
const ROWS_WINDOW: &str = "export component RowsWindow inherits Window {\n\
     in property <[{id: string}]> rows;\n    width: 200px;\n    height: 100px;\n\
     VerticalLayout { for row in rows: Text { text: row.id; } }\n}\n";

/// The UI files every package carries.
const UI: [(&str, &str); 4] = [
    ("ui/list.slint", PANEL),
    ("ui/win.slint", WINDOW),
    ("ui/rows.slint", ROWS),
    ("ui/rows-win.slint", ROWS_WINDOW),
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

/// A development plugin `id` with `body` under `root/development`.
fn development(root: &Path, id: &str, body: &str) -> PathBuf {
    let directory = root.join("development");
    package(&directory, id, body);
    directory.join(id)
}

/// `body` with each `(placeholder, value)` replaced.
fn fill(body: &str, values: &[(&str, String)]) -> String {
    values.iter().fold(body.to_owned(), |text, (key, value)| {
        text.replace(key, value)
    })
}

/// One offscreen run: its stderr and its wire trace.
struct Run {
    log: String,
    trace: Vec<Value>,
    trace_path: PathBuf,
}

impl Run {
    /// The values logged under `tag`, in order.
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
    seconds: i64,
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
        "VESTRA_TEST_KEY",
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

/// The stall env and its window in epoch ms (`begins`, `ends`), computed
/// from `started` (taken just before the player starts). The real stall
/// begins a little later (the player's own start, or its first tick after
/// the time asked), so it also ends a little later: "before `ends`" is
/// inside the stall, and "after `ends`" may be.
struct Stall {
    begins: i64,
    ends: i64,
}

impl Stall {
    fn new(started: i64) -> (Self, (&'static str, String)) {
        (
            Stall {
                begins: started + STALL_AT_MS,
                ends: started + STALL_AT_MS + STALL_MS,
            },
            ("VESTRA_TEST_UI_STALL", format!("{STALL_AT_MS},{STALL_MS}")),
        )
    }
}

/// The run's length for a stall run: the stall, then 4 s.
const STALL_RUN_SECONDS: i64 = (STALL_AT_MS + STALL_MS) / 1000 + 4;

/// Which requests of a method a test means.
type Matches = fn(&Value) -> bool;

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

fn ok() -> Value {
    json!({"ok": null})
}

// ---------------------------------------------------------------- R4 ----

const R4: &str = "test.spec-r4";

/// R4 (two streams, no queue; plan §4 "spec test: a UI-independent request
/// sent after a stalled UI-bound one is answered first"): a view is open;
/// during the stall the plugin sends `ui.set` on it (UI-bound) and then,
/// without awaiting it, three UI-independent requests (`storage.session.get`,
/// `storage.session.set`, `layout.state`). The three are answered during the
/// stall, and their replies are written before the `ui.set` reply.
///
/// After S3a. On today's code nothing is answered during the stall, and
/// after it the `ui.set` reply comes first: fails at "answered during the
/// stall".
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r4_a_ui_independent_request_after_a_stalled_ui_bound_one_is_answered_first() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let (stall, stall_env) = Stall::new(started);
    let plugin = development(
        root.path(),
        R4,
        &fill(
            r#"export default function activate(plugin) {
  setTimeout(async () => {
    log("r4-open", await req(plugin, "ui.open", { id: "v", file: "ui/win.slint", properties: { label: "a" }, callbacks: [] }));
    await until(__AT__);
    const sent = Date.now();
    const stamp = (promise) => promise.then((result) => ({ ...result, at: Date.now() }));
    const all = await Promise.all([
      stamp(req(plugin, "ui.set", { id: "v", property: "label", value: "b" })),
      stamp(req(plugin, "storage.session.get", { key: "r4-a" })),
      stamp(req(plugin, "storage.session.set", { key: "r4-b", value: 1 })),
      stamp(req(plugin, "layout.state")),
    ]);
    log("r4", { sent, bound: all[0], free: all.slice(1) });
  }, 1000);
}
"#,
            &[("__AT__", (stall.begins + 1_000).to_string())],
        ),
    );
    let run = run(root.path(), &[], &[plugin], STALL_RUN_SECONDS, &[stall_env]);
    assert_eq!(run.one("r4-open"), ok(), "{}", run.log);
    let result = run.one("r4");
    let sent = result["sent"].as_i64().unwrap();
    assert!(
        sent < stall.ends - 2_000,
        "the requests were sent during the stall"
    );
    assert!(result["bound"].get("ok").is_some(), "{result}");
    for free in result["free"].as_array().unwrap() {
        assert!(free.get("ok").is_some(), "{result}");
        let at = free["at"].as_i64().unwrap();
        assert!(
            at < stall.ends && at - sent < PROMPT_MS,
            "R4/R7: a UI-independent request answered during the stall \
             ({} ms after it was sent, {} ms before the stall ended): {result}",
            at - sent,
            stall.ends - at
        );
    }
    // The wire: each UI-independent reply before the UI-bound one.
    let records = run.records(R4);
    let set = request(&records, "ui.set", |p| p["value"] == "b");
    let set_reply = reply_to(&records, set);
    let free: [(&str, Matches); 3] = [
        ("storage.session.get", |p| p["key"] == "r4-a"),
        ("storage.session.set", |p| p["key"] == "r4-b"),
        ("layout.state", |_| true),
    ];
    for (method, matches) in free {
        let asked = request(&records, method, matches);
        assert!(seq(asked) > seq(set), "{method} sent after ui.set");
        let answered = reply_to(&records, asked);
        assert!(
            seq(answered) < seq(set_reply),
            "R4: the {method} reply (seq {}) before the stalled ui.set reply (seq {})",
            seq(answered),
            seq(set_reply)
        );
    }
}

const R4_ORDER: &str = "test.spec-r4-order";

/// R4, the UI-bound stream (design §5.1 "Order inside the UI-bound
/// stream"): a plugin's UI-bound requests are carried out in arrival
/// order, also when they all wait for a stalled UI. Sent during the stall
/// without awaiting: `ui.open`, `ui.set`, `ui.show`, `ui.close`, `ui.set`;
/// the last finds no view. Their replies keep the stream's order (these
/// five are answered as soon as the UI carried them out).
///
/// Holds today (all handled after the stall, in order) and on two threads.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r4_the_ui_bound_stream_keeps_arrival_order_across_a_stall() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let (stall, stall_env) = Stall::new(started);
    let plugin = development(
        root.path(),
        R4_ORDER,
        &fill(
            r#"export default function activate(plugin) {
  setTimeout(async () => {
    await until(__AT__);
    const open = (id) => req(plugin, "ui.open", { id, file: "ui/win.slint", properties: { label: id }, callbacks: [] });
    const set = (id, value) => req(plugin, "ui.set", { id, property: "label", value });
    log("r4-order", await Promise.all([
      open("b"),
      set("b", "z"),
      req(plugin, "ui.show", { id: "b", visible: false }),
      req(plugin, "ui.close", { id: "b" }),
      set("b", "after close"),
    ]));
  }, 1000);
}
"#,
            &[("__AT__", (stall.begins + 1_000).to_string())],
        ),
    );
    let run = run(root.path(), &[], &[plugin], STALL_RUN_SECONDS, &[stall_env]);
    assert_eq!(
        run.one("r4-order"),
        json!([ok(), ok(), ok(), ok(), {"error":"missing"}]),
        "R4: the UI-bound stream carries out a plugin's requests in arrival order"
    );
    let records = run.records(R4_ORDER);
    let methods = ["ui.open", "ui.set", "ui.show", "ui.close"];
    let replies = methods
        .iter()
        .map(|method| {
            let asked = records
                .iter()
                .find(|r| r["dir"] == "in" && r["line"]["method"] == *method)
                .unwrap_or_else(|| panic!("a {method} request"));
            seq(reply_to(&records, asked))
        })
        .collect::<Vec<_>>();
    assert!(
        replies.windows(2).all(|pair| pair[0] < pair[1]),
        "R4: replies of ui.open, ui.set, ui.show, ui.close in order: {replies:?}"
    );
}

// ---------------------------------------------------------------- R7 ----

const R7: &str = "test.spec-r7";

/// The UI-independent requests R7 sends (method table rows with stream I
/// that can be called offscreen, outside the start-up gate of note 6). Left
/// out, with reason: `system.*` (opens files or URLs on the desktop),
/// `secrets.*` (the system's credential store), `clipboard.write*` (the
/// user's clipboard), `rendering.restart` (restarts the player),
/// `rendering.setPreference`, `appearance.setLanguage` and `packages.*`
/// writes (persist outside the run), `host.run` (playback actions and
/// fullscreen), `sequence.*` and `sourceResolvers.*` (provider claims; the
/// start-up gate), `mpv.*` and the playback writes (`playback.set`,
/// `source.submit`, ...: no media in these runs), `producer.*`, `jobs.*`
/// and `snapshot.*` writes, `events.*` (the SDK keeps its own
/// subscriptions), `ui.menu` and `ui.interaction` (need a mounted panel),
/// `palette.lead.set` and `palette.lead.done` (need a palette ticket).
const R7_REQUESTS: &str = r#"const declaration = (title, bindings) => ({ title, path: [], order: 0, group: "main",
  radio: false, bindings, focus: null, repeat: false });
const state = { enabled: true, checked: null, busy: false, reason: null };
const requests = (tag) => [
  ["storage.session.set", { key: `r7-${tag}`, value: 1 }],
  ["storage.session.get", { key: `r7-${tag}` }],
  ["storage.session.keys", {}],
  ["storage.session.delete", { key: "r7-none" }],
  ["window.state"],
  ["layout.state"],
  ["bindings.list"],
  ["bindings.set", { plugin: "__ID__", command: "r7-cmd", inputs: ["Ctrl+Shift+F9"] }],
  ["bindings.reset", { plugin: "__ID__", command: "r7-cmd" }],
  ["commands.keys", {}],
  ["commands.list", {}],
  ["commands.register", { id: `r7-new-${tag}`, revision: 1, declaration: declaration("New", []), state }],
  ["commands.update", { id: "r7-cmd", revision: tag === "control" ? 2 : 3, state: { ...state, enabled: tag === "control" } }],
  ["commands.unregister", { id: `r7-gone-${tag}` }],
  ["osd.show", { text: `R7 ${tag}` }],
  ["osd.panel", { text: null }],
  ["appearance.set", { theme: tag === "control" ? "vestra-light" : "vestra-dark" }],
  ["window.setCursorAutoHide", { mode: tag === "control" ? "playing" : "fullscreen" }],
  ["window.setAlwaysOnTop", { onTop: false }],
  ["timeline.marks"],
  ["timeline.set", { marks: [] }],
  ["triggers.set", { trigger: `r7-${tag}`, when: { phase: ["playing"] } }],
  ["drops.listen", { listen: false }],
  ["packages.list", {}],
  ["rendering.state"],
  ["app.system"],
  ["ui.markdownScan", { text: "R7 *text*", base: null }],
  ["settings.unregister", { id: "r7-none" }],
  ["palette.lead.clear", {}],
  ["clipboard.readText"],
  ["source.state"],
  ["playback.stats"],
  ["processing.state"],
  ["subtitles.state"],
  ["player.chapters"],
  ["player.audioDevices"],
  ["video.upscaling.state"],
  ["media.sources.list", {}],
];
const battery = async (tag) => {
  const sent = Date.now();
  const results = await Promise.all(requests(tag).map(([method, params]) =>
    req(plugin, method, params).then((result) => ({
      method,
      took: Date.now() - sent,
      thrown: result.thrown,
      error: result.error,
      // Only what the test reads: never the clipboard's text.
      displays: method === "app.system" ? (result.ok?.displays?.length ?? null) : undefined,
    }))));
  log(`r7-${tag}`, { sent, results });
};
"#;

/// R7 (a UI-independent request never waits for the UI; plan S3a "Tests to
/// add": "no UI-independent method waits during a stall"): the same
/// requests (`R7_REQUESTS`: core-only reads, the "touches the interface,
/// reply independent" ones such as `osd.show`, `bindings.set`,
/// `commands.register`, `appearance.set`, and worker-forwarded reads),
/// sent at once, are each answered within `PROMPT_MS` outside the stall
/// (the control) and during it. `app.system` lists displays when answered
/// on the core (groups A, G; design §5.1 note 10).
///
/// After S3a. On today's code the control holds and the stall battery
/// fails at "R7: ... waited" (every reply takes about the stall's rest).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r7_no_ui_independent_method_waits_during_a_ui_stall() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let (stall, stall_env) = Stall::new(started);
    let body = fill(
        &format!(
            r#"export default function activate(plugin) {{
{R7_REQUESTS}
  setTimeout(async () => {{
    const reg = (id) => req(plugin, "commands.register", {{ id, revision: 1, declaration: declaration(id, []), state }});
    log("r7-setup", await Promise.all(["r7-cmd", "r7-gone-control", "r7-gone-stall"].map(reg)));
    await until(__CONTROL__);
    await battery("control");
    await until(__AT__);
    await battery("stall");
  }}, 1000);
}}
"#
        ),
        &[
            ("__ID__", R7.to_owned()),
            ("__CONTROL__", (stall.begins - 2_500).to_string()),
            ("__AT__", (stall.begins + 1_000).to_string()),
        ],
    );
    let plugin = development(root.path(), R7, &body);
    let run = run(root.path(), &[], &[plugin], STALL_RUN_SECONDS, &[stall_env]);
    let setup = run.one("r7-setup");
    assert!(
        setup
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("ok").is_some()),
        "{setup}"
    );
    let check = |tag: &str, before: Option<i64>| {
        let battery = run.one(&format!("r7-{tag}"));
        let sent = battery["sent"].as_i64().unwrap();
        if let Some(before) = before {
            assert!(
                sent < before - 2_000,
                "the {tag} battery was sent during the stall"
            );
        }
        for result in battery["results"].as_array().unwrap() {
            let method = &result["method"];
            assert!(
                result["thrown"].is_null(),
                "{method} was sent (the SDK refused it: {})",
                result["thrown"]
            );
            let took = result["took"].as_i64().unwrap();
            assert!(
                took < PROMPT_MS && before.is_none_or(|before| sent + took < before),
                "R7: {method} waited {took} ms ({tag}; error {}):\n{battery:#}",
                result["error"]
            );
        }
        let system = battery["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["method"] == "app.system")
            .unwrap();
        system["displays"].clone()
    };
    let control = check("control", None);
    let stalled = check("stall", Some(stall.ends));
    // Offscreen lists no displays today (see the groups A/G test), so this
    // only says the core's copy is the same during the stall.
    assert_eq!(
        stalled, control,
        "app.system lists the same displays during the stall (groups A, G)"
    );
}

// ---------------------------------------------------------------- R8 ----

const R8: &str = "test.spec-r8";

/// R8 (keys never judged by an older table; S3a brief: "both table
/// snapshots (bindings, registry) waiting in the queue are applied before
/// any key event behind them is judged"). The plugin's command `old` is
/// bound to F7 by default and `new` has no key. The plugin then rebinds
/// (`bindings.set`: `old` to nothing, `new` to F7) and registers `late` with
/// F8 (`commands.register`), each awaited; then F7 and F8 are pressed on
/// the UI side. Expected: `new` and `late` run, `old` never. Once without a
/// stall, and once with the rebinding answered during a stall (two threads:
/// the core answers at once, the snapshots wait in the UI's queue) and the
/// keys pressed while the UI is still held, so they are judged only after
/// the stall, behind the snapshots.
///
/// **Needs a test hook the host does not have (plan: "a UI-side key
/// injection hook, test-only"), proposed:** `VESTRA_TEST_KEY=<binding>@<ms>[,...]`
/// (binding syntax as `bindings.set`; ms after the player started): a
/// test-only thread posts each key at its time to the UI's event loop
/// (`slint::invoke_from_event_loop`, the queue the core's commands use), and
/// the UI handles it as the main window's key callback does, judged against
/// the UI's snapshot of the tables (the path of `InjectKey`). Posted from
/// another thread, a key at a time inside the stall queues behind the
/// snapshots the core sent before it, which is the case R8 is about. No
/// change to any other path.
///
/// After S3a and the hook. On today's code (no hook) no key arrives: fails
/// at "F7 ran a command".
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r8_a_key_is_judged_by_the_tables_queued_before_it() {
    for stalled in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let started = epoch_ms();
        let (stall, stall_env) = Stall::new(started);
        // The rebinding at `rebind`; the keys 1.5 s later (inside the
        // stall when stalled, so judged after it).
        let rebind = started + if stalled { STALL_AT_MS + 500 } else { 3_000 };
        let keys_at = rebind - started + 1_500;
        let plugin = development(
            root.path(),
            R8,
            &fill(
                r#"export default function activate(plugin) {
  plugin.commands.add({ id: "old", title: "Old", bindings: ["F7"], run: () => log("r8-ran", "old") });
  plugin.commands.add({ id: "new", title: "New", run: () => log("r8-ran", "new") });
  void (async () => {
    await until(__REBIND__);
    const sent = Date.now();
    const results = [
      await req(plugin, "bindings.set", { plugin: "__ID__", command: "old", inputs: [] }),
      await req(plugin, "bindings.set", { plugin: "__ID__", command: "new", inputs: ["F7"] }),
      await req(plugin, "commands.register", { id: "late", revision: 1,
        declaration: { title: "Late", path: [], order: 0, group: "main", radio: false, bindings: ["F8"], focus: null, repeat: false },
        state: { enabled: true, checked: null, busy: false, reason: null } }),
    ];
    log("r8-rebound", { sent, at: Date.now(), results });
  })();
}
"#,
                &[
                    ("__ID__", R8.to_owned()),
                    ("__REBIND__", rebind.to_string()),
                ],
            ),
        );
        let mut env = vec![(
            "VESTRA_TEST_KEY",
            format!("F7@{keys_at},F8@{}", keys_at + 100),
        )];
        if stalled {
            env.push(stall_env);
        }
        let seconds = if stalled { STALL_RUN_SECONDS } else { 8 };
        let run = run(root.path(), &[], &[plugin], seconds, &env);
        let case = if stalled { "stalled" } else { "control" };
        let ran = run.spec("r8-ran");
        assert!(
            !ran.is_empty(),
            "{case}: F7 ran a command (the key injection hook exists): {}",
            run.log
        );
        let rebound = run.one("r8-rebound");
        assert!(
            rebound["results"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r.get("ok").is_some()),
            "{case}: {rebound}"
        );
        assert!(
            rebound["at"].as_i64().unwrap() < started + keys_at,
            "{case}: the tables changed before the keys were pressed: {rebound}"
        );
        assert_eq!(
            ran,
            vec![json!("new")],
            "R8 ({case}): F7 is judged by the table that preceded it"
        );
        let records = run.records(R8);
        let late = records
            .iter()
            .filter(|r| {
                r["dir"] == "out"
                    && r["line"]["event"] == "commands.execute"
                    && r["line"]["data"]["id"] == "late"
            })
            .count();
        assert_eq!(
            late, 1,
            "R8 ({case}): F8 runs the command registered before it"
        );
        if stalled {
            assert!(
                rebound["at"].as_i64().unwrap() < stall.ends,
                "the rebinding was answered during the stall (R7): {rebound}"
            );
        }
    }
}

// --------------------------------------------------------------- R13 ----

const R13_MOUNT: &str = "test.spec-r13-mount";

/// Reads `layout.state` `count` times, `gap` ms apart (R13: the core's copy
/// never moves backwards once the reply was sent).
const READS: &str = r#"const reads = async (count, gap) => {
  const all = [];
  for (let i = 0; i < count; i++) {
    all.push(await req(plugin, "layout.state"));
    await pause(gap);
  }
  return all;
};
"#;

/// R13 (state, then the reply, then the broadcast) for `ui.mount` sent
/// during a stall: the reply comes when the UI applied the mount (after the
/// stall); `layout.state` sent right after the reply lists the panel, and
/// so do ten more reads over the next 300 ms (the core's copy is never
/// older than the reply: the facts' sequence number, R13). On the wire no
/// `window.state` between the request and its reply; one after it (the
/// sidebar took its width). R5: `ui.preparing` before the reply.
///
/// Holds today and on two threads.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r13_a_mount_answered_after_a_stall_is_in_place_when_answered() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let (stall, stall_env) = Stall::new(started);
    let plugin = development(
        root.path(),
        R13_MOUNT,
        &fill(
            &format!(
                r#"export default function activate(plugin) {{
  plugin.wire.on("window.state", () => {{}});
{READS}
  setTimeout(async () => {{
    log("r13m-before", await req(plugin, "layout.state"));
    await until(__AT__);
    const sent = Date.now();
    const mount = await req(plugin, "ui.mount", {{ slot: "sidebar", file: "ui/list.slint",
      component: "List", global: "Bridge", title: "Spec", properties: {{ tabbed: true }} }});
    log("r13m-mount", {{ ...mount, sent, at: Date.now() }});
    log("r13m-after", await reads(11, 30));
  }}, 1000);
}}
"#
            ),
            &[("__AT__", (stall.begins + 1_000).to_string())],
        ),
    );
    let run = run(root.path(), &[], &[plugin], STALL_RUN_SECONDS, &[stall_env]);
    let panels = |layout: &Value| {
        layout["ok"]["sidebar"]["panels"]
            .as_array()
            .map(|panels| panels.iter().filter(|p| p["plugin"] == R13_MOUNT).count())
            .unwrap_or(0)
    };
    let before = run.one("r13m-before");
    assert_eq!(panels(&before), 0, "no panel before: {before}");
    let mount = run.one("r13m-mount");
    assert!(mount.get("ok").is_some(), "{mount}");
    assert!(
        mount["sent"].as_i64().unwrap() < stall.ends - 2_000,
        "ui.mount was sent during the stall"
    );
    for (index, read) in run.one("r13m-after").as_array().unwrap().iter().enumerate() {
        assert_eq!(
            panels(read),
            1,
            "R13: read {index} after the ui.mount reply lists the panel: {read}"
        );
    }
    let records = run.records(R13_MOUNT);
    let asked = request(&records, "ui.mount", |p| p["slot"] == "sidebar");
    let answered = reply_to(&records, asked);
    let owner = &asked["owner"];
    let preparing = events(&records, owner, "ui.preparing");
    assert!(
        !preparing.is_empty() && preparing.iter().all(|e| seq(e) < seq(answered)),
        "R5: ui.preparing before the ui.mount reply"
    );
    let states = events(&records, owner, "window.state");
    assert_eq!(
        states
            .iter()
            .filter(|e| seq(e) > seq(asked) && seq(e) < seq(answered))
            .count(),
        0,
        "R13: no window.state between ui.mount and its reply"
    );
    assert!(
        states.iter().any(|e| seq(e) > seq(answered)),
        "R13: a window.state follows the reply"
    );
}

const R13_LAYOUT: &str = "test.spec-r13-layout";

/// R13 with its `layout.set` exception, across a stall: a bottom panel is
/// mounted before the stall; during it the plugin sends `layout.set`
/// (hide the bottom bar). When answered, `layout.state` read right after
/// shows it hidden, and ten more reads over 300 ms agree (never back). On
/// the wire a `window.state` is written between the request and its reply
/// (the exception: today's order, kept through S3a); then, after the
/// stall, the same with `visible: true`.
///
/// Holds today and on two threads.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn r13_a_layout_change_answered_after_a_stall_is_in_place_when_answered() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let (stall, stall_env) = Stall::new(started);
    let plugin = development(
        root.path(),
        R13_LAYOUT,
        &fill(
            &format!(
                r#"export default function activate(plugin) {{
  plugin.wire.on("window.state", () => {{}});
{READS}
  setTimeout(async () => {{
    log("r13l-mount", await req(plugin, "ui.mount", {{ slot: "bottom", file: "ui/list.slint",
      component: "List", global: "Bridge", title: "Spec", properties: {{}} }}));
    log("r13l-before", await req(plugin, "layout.state"));
    await until(__AT__);
    const sent = Date.now();
    const hide = await req(plugin, "layout.set", {{ slot: "bottom", visible: false }});
    log("r13l-hide", {{ ...hide, sent }});
    log("r13l-hidden", await reads(11, 30));
    await pause(500);
    log("r13l-show", await req(plugin, "layout.set", {{ slot: "bottom", visible: true }}));
    log("r13l-shown", await reads(11, 30));
  }}, 1000);
}}
"#
            ),
            &[("__AT__", (stall.begins + 1_000).to_string())],
        ),
    );
    let run = run(root.path(), &[], &[plugin], STALL_RUN_SECONDS, &[stall_env]);
    assert!(run.one("r13l-mount").get("ok").is_some(), "{}", run.log);
    let before = run.one("r13l-before");
    assert_eq!(before["ok"]["bottom"]["preferred"], true, "{before}");
    let hide = run.one("r13l-hide");
    assert!(hide.get("ok").is_some(), "{hide}");
    assert!(
        hide["sent"].as_i64().unwrap() < stall.ends - 2_000,
        "layout.set was sent during the stall"
    );
    assert!(run.one("r13l-show").get("ok").is_some());
    for (tag, preferred) in [("r13l-hidden", false), ("r13l-shown", true)] {
        for (index, read) in run.one(tag).as_array().unwrap().iter().enumerate() {
            assert_eq!(
                read["ok"]["bottom"]["preferred"], preferred,
                "R13: {tag} read {index} after the layout.set reply: {read}"
            );
        }
    }
    let records = run.records(R13_LAYOUT);
    for visible in [false, true] {
        let asked = request(&records, "layout.set", |p| p["visible"] == visible);
        let answered = reply_to(&records, asked);
        let between = events(&records, &asked["owner"], "window.state")
            .into_iter()
            .filter(|e| seq(e) > seq(asked) && seq(e) < seq(answered))
            .count();
        assert!(
            between >= 1,
            "R13's layout.set exception: window.state (visible {visible}) before the reply"
        );
    }
}

// ---------------------------------------------------------------- K8 ----

/// K8 (`ResetWrites`, `SetMount`, `SettingsOp::Set` and `ViewOp` travel in
/// one ordered stream, so a write is judged against the records the reset
/// before it left; design K8 "Carried forward": the
/// initial properties of `ui.open`, `ui.mount` and `settings.register` are
/// write 0 and those calls reset the record; a patch whose `base` differs
/// from the record is `stale`).
///
/// After a setup (a tool window open, a sidebar panel mounted, a settings
/// page registered, each awaited) one sequence mixes the three kinds:
/// a whole write numbered, a patch on it, the reset (`settings.unregister`
/// then `settings.register`; `ui.close` then `ui.open`; `ui.unmount` then
/// `ui.mount`: today a second `ui.mount` of a mounted slot is `conflict`
/// and a second `settings.register` of a registered page `invalid`), a
/// patch on the old number (`stale`), a patch on 0 (`ok`). Steps marked
/// `true` are awaited in every mode, after everything sent before them:
/// the settings reset is answered by the core (stream I), so sent
/// unawaited it may overtake the earlier `settings.set` (R4 allows that;
/// it is not what K8 is about), and `ui.unmount`/`ui.mount` are answered
/// after the mount batch. The settings group comes first so that in the
/// `stall` mode its reset is answered while the UI is held (two threads)
/// and the patches behind it wait in the UI's stream.
const K8_STEPS: &str = r#"const insert = { op: "insert", at: 0, rows: [{ id: "x" }] };
const patch = (base, seq) => ({ patch: { base, seq, ops: [insert] } });
const view = { id: "k8-__MODE__", file: "ui/rows-win.slint", properties: { rows: [] }, callbacks: [] };
const mount = { slot: "sidebar", file: "ui/rows.slint", component: "RowsPanel", global: "Rows",
  title: "K8", properties: { rows: [] } };
const page = { id: "k8-__MODE__", category: "plugins", title: "K8 __MODE__", file: "ui/rows.slint",
  component: "RowsPanel", global: "Rows", properties: { rows: [] } };
const setup = [["ui.open", view], ["ui.mount", mount], ["settings.register", page]];
const steps = [
  ["settings.set", { id: page.id, property: "rows", value: [{ id: "a" }], seq: 2 }],
  ["settings.set", { id: page.id, property: "rows", ...patch(2, 3) }],
  ["settings.unregister", { id: page.id }, true],
  ["settings.register", page, true],
  ["settings.set", { id: page.id, property: "rows", ...patch(3, 4) }],
  ["settings.set", { id: page.id, property: "rows", ...patch(0, 5) }],
  ["ui.set", { id: view.id, property: "rows", value: [{ id: "a" }, { id: "b" }], seq: 5 }],
  ["ui.set", { id: view.id, property: "rows", ...patch(5, 6) }],
  ["ui.close", { id: view.id }],
  ["ui.open", view],
  ["ui.set", { id: view.id, property: "rows", ...patch(6, 7) }],
  ["ui.set", { id: view.id, property: "rows", ...patch(0, 8) }],
  ["ui.setMount", { slot: "sidebar", property: "rows", value: [{ id: "a" }], seq: 2 }],
  ["ui.setMount", { slot: "sidebar", property: "rows", ...patch(2, 3) }],
  ["ui.unmount", { slot: "sidebar" }, true],
  ["ui.mount", mount, true],
  ["ui.setMount", { slot: "sidebar", property: "rows", ...patch(3, 4) }],
  ["ui.setMount", { slot: "sidebar", property: "rows", ...patch(0, 5) }],
];
const code = (result) => result.thrown ? `thrown ${result.thrown}` : (result.error ?? "ok");
"#;

/// The codes `K8_STEPS` must give, in order.
const K8_EXPECTED: [&str; 18] = [
    "ok", "ok", "ok", "ok", "stale", "ok", // the settings page
    "ok", "ok", "ok", "ok", "stale", "ok", // the tool window
    "ok", "ok", "ok", "ok", "stale", "ok", // the panel
];

/// K8: the sequence gives the same codes whether each request is awaited
/// (`awaited`), sent at once without awaiting but for the marked steps
/// (`burst`), or the same during a UI stall (`stall`: the core answers the
/// reply-independent ones, `settings.register` among them, while the UI
/// is held, and the UI-bound ones wait in the one stream behind the resets
/// sent before them). Three development plugins, one per mode, in one run.
///
/// Holds today and on two threads.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn k8_resets_and_writes_keep_one_order() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let (stall, stall_env) = Stall::new(started);
    let modes = [
        ("awaited", started + 2_500),
        ("burst", started + 3_500),
        ("stall", stall.begins + 1_000),
    ];
    let plugins = modes
        .iter()
        .map(|(mode, at)| {
            development(
                root.path(),
                &format!("test.spec-k8-{mode}"),
                &fill(
                    &format!(
                        r#"export default function activate(plugin) {{
{K8_STEPS}
  setTimeout(async () => {{
    const ready = [];
    for (const [method, params] of setup) ready.push(code(await req(plugin, method, params)));
    log("k8-__MODE__-setup", ready);
    await until(__AT__);
    let codes;
    if ("__MODE__" === "awaited") {{
      codes = [];
      for (const [method, params] of steps) codes.push(code(await req(plugin, method, params)));
    }} else {{
      const results = [];
      let pending = [];
      for (const [method, params, wait] of steps) {{
        if (wait) {{
          results.push(...await Promise.all(pending));
          pending = [];
          results.push(await req(plugin, method, params));
        }} else pending.push(req(plugin, method, params));
      }}
      results.push(...await Promise.all(pending));
      codes = results.map(code);
    }}
    log("k8-__MODE__", codes);
  }}, 1000);
}}
"#
                    ),
                    &[("__MODE__", (*mode).to_owned()), ("__AT__", at.to_string())],
                ),
            )
        })
        .collect::<Vec<_>>();
    let run = run(root.path(), &[], &plugins, STALL_RUN_SECONDS, &[stall_env]);
    for (mode, _) in modes {
        assert_eq!(
            run.one(&format!("k8-{mode}-setup")),
            json!(["ok", "ok", "ok"]),
            "{mode}: the setup:\n{}",
            run.log
        );
        assert_eq!(
            run.one(&format!("k8-{mode}")),
            json!(K8_EXPECTED),
            "K8 ({mode}): a write is judged against the records the reset before it left"
        );
    }
}

// --------------------------------------------------------- Design §5.1 ----

const SET_MOUNT: &str = "test.spec-set-mount";

/// Design §5.1 "`ui.setMount` and an earlier pending `ui.mount` (kept)":
/// a `ui.setMount` applies to the plugin's mount that is ready when it
/// arrives, else it is refused `missing`. Pinned here, each set sent
/// without awaiting right after a `ui.mount`:
///
/// 1. no mount ready yet: `ui.mount` of the sidebar and `ui.setMount` of
///    the sidebar: the set is `missing`, the mount succeeds;
/// 2. the sidebar's mount ready: `ui.mount` of the bottom bar, then
///    `ui.setMount` of the bottom bar (`missing`: that mount is not ready
///    yet) and of the sidebar (succeeds, on the ready mount).
///
/// Once outside a stall, and again sent during a stall (both panels
/// unmounted before it), with the same outcome. Holds today and on two
/// threads.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn design_5_1_a_set_mount_before_a_pending_mount_keeps_today_s_outcome() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let (stall, stall_env) = Stall::new(started);
    let plugin = development(
        root.path(),
        SET_MOUNT,
        &fill(
            r#"export default function activate(plugin) {
  const mount = (slot) => req(plugin, "ui.mount", { slot, file: "ui/list.slint", component: "List",
    global: "Bridge", title: slot, properties: { tabbed: true } });
  const set = (slot) => req(plugin, "ui.setMount", { slot, property: "tabbed", value: false });
  const cases = async (tag) => {
    log(`sm-${tag}-first`, await Promise.all([mount("sidebar"), set("sidebar")]));
    log(`sm-${tag}-other`, await Promise.all([mount("bottom"), set("bottom"), set("sidebar")]));
  };
  setTimeout(async () => {
    await cases("plain");
    log("sm-unmount", await Promise.all([
      req(plugin, "ui.unmount", { slot: "sidebar" }), req(plugin, "ui.unmount", { slot: "bottom" })]));
    await until(__AT__);
    await cases("stall");
  }, 1500);
}
"#,
            &[("__AT__", (stall.begins + 1_000).to_string())],
        ),
    );
    let run = run(root.path(), &[], &[plugin], STALL_RUN_SECONDS, &[stall_env]);
    let unmount = run.one("sm-unmount");
    assert!(
        unmount
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("ok").is_some()),
        "{unmount}"
    );
    let missing = json!({"error":"missing"});
    for tag in ["plain", "stall"] {
        let first = run.one(&format!("sm-{tag}-first"));
        assert!(first[0].get("ok").is_some(), "{tag}: {first}");
        assert_eq!(
            first[1], missing,
            "§5.1 ({tag}): no ready mount yet, ui.setMount refused: {first}"
        );
        let other = run.one(&format!("sm-{tag}-other"));
        assert!(other[0].get("ok").is_some(), "{tag}: {other}");
        assert_eq!(
            other[1], missing,
            "§5.1 ({tag}): the bottom bar's mount is not ready yet: {other}"
        );
        assert_eq!(
            other[2],
            ok(),
            "§5.1 ({tag}): ui.setMount applies to the mount ready when it arrives: {other}"
        );
    }
}

// ------------------------------------------------------ Groups A and G ----

const SYSTEM: &str = "test.spec-system";

/// Groups A and G (design §5.3; §5.1 note 10; S3a brief "app.system answers
/// from the Displays copy"): `app.system` lists at least one display. On
/// two threads it is answered on the core, from the displays the interface
/// reported; a core-thread call into window code would see none (risk 2).
/// Asked once early and once late in the run.
///
/// **Not testable offscreen today:** the offscreen run answers `app.system`
/// with no displays (measured 2026-11-16 on main 4be4db59: `displays: []`),
/// so "non-empty" cannot be seen through the public harness. Smallest hook,
/// proposed: on the offscreen platform the interface still sends its
/// `UiReport::Displays` from the system's display list (Win32
/// `EnumDisplayMonitors`, `NSScreen`; no window needed), or a test switch
/// `VESTRA_TEST_DISPLAYS=<w>x<h>[,...]` that makes it report those. With
/// the hook this test holds today and on two threads. (The R7 test checks
/// that the stall's answer lists as many displays as the control's.)
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn groups_a_g_app_system_lists_the_displays() {
    let root = tempfile::tempdir().unwrap();
    let plugin = development(
        root.path(),
        SYSTEM,
        r#"export default function activate(plugin) {
  const displays = async () => {
    const result = await req(plugin, "app.system");
    return result.ok ? { count: result.ok.displays?.length ?? 0 } : result;
  };
  setTimeout(async () => {
    log("system-early", await displays());
    await pause(3000);
    log("system-late", await displays());
  }, 1000);
}
"#,
    );
    let run = run(
        root.path(),
        &[],
        &[plugin],
        7,
        &[("VESTRA_TEST_DISPLAYS", "1920x1080".to_owned())],
    );
    for tag in ["system-early", "system-late"] {
        let result = run.one(tag);
        assert!(
            result["count"].as_u64().unwrap_or(0) > 0,
            "{tag}: app.system lists the displays: {result}"
        );
    }
}

// ---------------------------------------------------------------- K9 ----

const K9: &str = "test.spec-k9";

/// The run's length in the K9 test: it ends inside the stall.
const K9_SECONDS: i64 = 9;
const K9_STALL_MS: i64 = 8_000;

/// K9 and risk 5 (at quit every process gets its stop at
/// once and what is left is killed after 1 s; S3a: "the core has its own
/// deadline and exit ... also when the UI never asks"; plan §4: "the plugin
/// shutdown deadline holds while the UI part is stalled"): the run ends
/// (`--seconds 9`) while the UI is stalled (6 s to 14 s). The plugin is
/// told to stop within the budget: its `onDispose` runs at most 1 s after
/// the run's end, well before the stall ends.
///
/// The run's end is counted from the host's first phase, not from the
/// test's clock: `--seconds` counts from `start` in `main.rs`, taken when
/// start-up is done, so a slower start-up (a loaded machine) moved the end
/// and failed the test without the stop being late. The test cannot see
/// `start`; it sees the plugin's activation, which comes right after the
/// host's first phase and always after `start` (the core thread that
/// activates plugins starts after it). So the run ends at most
/// `K9_SECONDS` after the activation, and the stop must come at most 1 s
/// after that. The stall bound is unchanged: the stall is timed from the
/// process start, which `started` precedes.
///
/// Assumes the run's end is a quit the core acts on by itself (on two
/// threads `--seconds` must not wait for the UI). If S3a keeps that clock
/// on the UI, this test needs a hook instead, proposed:
/// `VESTRA_TEST_QUIT=<ms>`: the core asks itself to quit at `<ms>` after the
/// start, as the host's Quit command does.
///
/// After S3a. On today's code the stop waits for the stall: fails at
/// "K9: the plugin was told to stop".
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn k9_the_plugin_shutdown_deadline_holds_with_the_ui_stalled() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let plugin = development(
        root.path(),
        K9,
        r#"export default function activate(plugin) {
  log("k9-start", { at: Date.now() });
  plugin.onDispose((info) => log("k9-dispose", { at: Date.now(), reason: info?.reason ?? null }));
  setTimeout(async () => log("k9-alive", await req(plugin, "storage.session.get", { key: "k" })), 1000);
}
"#,
    );
    let run = run(
        root.path(),
        &[],
        &[plugin],
        K9_SECONDS,
        &[(
            "VESTRA_TEST_UI_STALL",
            format!("{STALL_AT_MS},{K9_STALL_MS}"),
        )],
    );
    assert!(run.one("k9-alive").get("ok").is_some(), "{}", run.log);
    let stall_ends = started + STALL_AT_MS + K9_STALL_MS;
    // At most the run's end: the activation is after `start` (above).
    let end = run.one("k9-start")["at"].as_i64().unwrap() + K9_SECONDS * 1_000;
    let disposed = run.spec("k9-dispose");
    let at = disposed
        .first()
        .and_then(|d| d["at"].as_i64())
        .unwrap_or_else(|| panic!("the plugin's onDispose ran: {}", run.log));
    assert!(
        at < end + 1_000 && at < stall_ends - 1_000,
        "K9: the plugin was told to stop {} ms after the run's end ({} ms before the stall ended): {}",
        at - end,
        stall_ends - at,
        run.log
    );
}

// ------------------------------------------------------------ Risk 3 ----

const TAB_ONE: &str = "vestra.fixture-spec-tab-one";
const TAB_TWO: &str = "vestra.fixture-spec-tab-two";
const TAB_QUIET: &str = "vestra.fixture-spec-tab-quiet";

/// A built-in that mounts a sidebar panel (when `panel`), is used until
/// `quiet` (epoch ms), then not; logs the sidebar's tabs just before.
fn tab_plugin(panel: bool, quiet: i64) -> String {
    fill(
        r#"export default async function activate(plugin) {
  if (__PANEL__)
    log("tab-mount-__ID__", await req(plugin, "ui.mount", { slot: "sidebar", file: "ui/list.slint",
      component: "List", global: "Bridge", title: "__ID__", properties: {} }));
  void (async () => {
    while (Date.now() < __QUIET__ - 300) {
      await plugin.storage.session.get("k");
      await pause(100);
    }
    const layout = await req(plugin, "layout.state");
    log("tab-quiet-__ID__", { at: Date.now(), sidebar: layout.ok?.sidebar ?? null });
  })();
}
"#,
        &[
            ("__PANEL__", panel.to_string()),
            ("__QUIET__", quiet.to_string()),
        ],
    )
}

/// Risk 3 (S3a brief: "`Busy::ShownTab` across a stall":
/// a plugin whose sidebar tab is the current one is busy). Two
/// built-ins mount a sidebar panel each, a third mounts none; all three go
/// quiet 400 ms before a UI stall, so a lazy first run is due to park 600
/// ms into it. On two threads the core parks during the
/// stall from its copy of the shown tab, which may be one report old: the
/// plugin whose tab is the current one must not park, during the stall or
/// after it. The other two park (the control: parking does happen).
///
/// Holds today (parks after the stall) and on two threads.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn risk_3_the_shown_tab_keeps_its_plugin_up_across_a_stall() {
    let root = tempfile::tempdir().unwrap();
    let started = epoch_ms();
    let (stall, stall_env) = Stall::new(started);
    let builtins = root.path().join("builtins");
    let quiet = stall.begins - 400;
    let entries = [(TAB_ONE, true), (TAB_TWO, true), (TAB_QUIET, false)]
        .iter()
        .map(|(id, panel)| {
            package(
                &builtins,
                id,
                &tab_plugin(*panel, quiet).replace("__ID__", id),
            )
        })
        .collect::<Vec<_>>();
    let run = run(root.path(), &entries, &[], STALL_RUN_SECONDS, &[stall_env]);
    for id in [TAB_ONE, TAB_TWO] {
        assert!(
            run.one(&format!("tab-mount-{id}")).get("ok").is_some(),
            "{id} mounted: {}",
            run.log
        );
    }
    let parked = |id: &str| run.log.contains(&format!("lazy: {id} parked"));
    // The current tab: the panel of the sidebar's `owner` (the wire form of
    // `layout.state`; the SDK's `active`).
    let sidebar = run.one(&format!("tab-quiet-{TAB_ONE}"))["sidebar"].clone();
    let active = sidebar["panels"]
        .as_array()
        .unwrap_or_else(|| panic!("the sidebar's tabs: {sidebar}"))
        .iter()
        .filter(|t| t["owner"] == sidebar["owner"])
        .map(|t| t["plugin"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(active.len(), 1, "one current tab: {sidebar}");
    let shown = active[0].as_str();
    let hidden = if shown == TAB_ONE { TAB_TWO } else { TAB_ONE };
    assert!(
        parked(TAB_QUIET),
        "the control without a panel parked: {}",
        run.log
    );
    assert!(
        parked(hidden),
        "the plugin of the other tab parked: {}",
        run.log
    );
    assert!(
        !parked(shown),
        "risk 3: {shown}, whose tab is the current one, never parked: {}",
        run.log
    );
}

// ---------------------------------------------------------------- B3 ----

/// S3a(3) review B3: a confirmation admitted while another is on its way to
/// the interface (issued, its sheet not yet shown) would replace the first
/// on the sheet and lose its reply. Two `dialogs.confirm` sent at once, not
/// awaited: the second is refused `busy`, and the first stays on the sheet
/// (unanswered for the rest of the run). Holds on one thread and on two.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn b3_a_second_confirmation_sent_at_once_is_refused_busy() {
    let root = tempfile::tempdir().unwrap();
    let plugin = development(
        root.path(),
        "test.spec-b3",
        r#"export default function activate(plugin) {
  setTimeout(async () => {
    const first = req(plugin, "dialogs.confirm", { message: "First?", confirm: "Yes", cancel: "No" });
    const second = req(plugin, "dialogs.confirm", { message: "Second?", confirm: "Yes", cancel: "No" });
    log("b3-second", await second);
    log("b3-first", await Promise.race([
      first.then(() => "answered"),
      pause(1500).then(() => "pending"),
    ]));
  }, 1500);
}
"#,
    );
    let run = run(root.path(), &[], &[plugin], 6, &[]);
    assert_eq!(run.one("b3-second")["error"], "busy", "{}", run.log);
    assert_eq!(
        run.one("b3-first"),
        json!("pending"),
        "the first stays on the sheet\n{}",
        run.log
    );
}

// ------------------------------------------------------- Core panic ----

/// S3a(4) review F1: a panic of the core on its thread ends the player
/// with a failure, as the same panic did on the UI thread before; the run
/// does not go on with a window and a dead core until its quit. Here the
/// core panics 1.5 s in (`VESTRA_TEST_CORE_PANIC`) of a 30 s run, which
/// must end within 10 s, not with a success.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_panic_of_the_core_ends_the_player_with_a_failure() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("builtins");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("builtin.json"),
        json!({"schema":3,"plugins":[]}).to_string(),
    )
    .unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    isolation::isolate(&mut command, root.path());
    command
        .args(["--silent", "--seconds", "30"])
        .env("VESTRA_TEST_OFFSCREEN", "1")
        .env("VESTRA_BUILTIN_PLUGINS", &directory)
        .env("VESTRA_INSTANCE_DIR", root.path().join("instance"))
        .env("VESTRA_LANGUAGE", "en")
        .env("VESTRA_TEST_CORE_PANIC", "1500")
        .stdout(std::process::Stdio::null())
        .stderr(std::fs::File::create(root.path().join("stderr.txt")).unwrap());
    let started = std::time::Instant::now();
    let mut child = command.spawn().unwrap();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > std::time::Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the run went on 10 s after the core's panic");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let log = std::fs::read_to_string(root.path().join("stderr.txt")).unwrap_or_default();
    assert_eq!(status.code(), Some(101), "the panic exit code: {log}");
    assert!(
        log.contains("VESTRA_TEST_CORE_PANIC"),
        "the core panicked: {log}"
    );
}
