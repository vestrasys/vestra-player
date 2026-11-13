//! The palette lead and `commands.run` with the real host, headless
//! (doc/architecture/palette-lead.md): a built-in fixture holds the command
//! palette's first row, gets the typed query with a ticket when the row is
//! activated (`VESTRA_TEST_EXECUTE=lead:<query>@<ms>`, as Enter would), runs
//! its own command with it (the OSD line names it), a destructive one after
//! the host's confirmation, and hits the ticket's limits; a development load
//! (a third-party plugin that declares `plugins`) may set the row first and
//! loses it to the built-in, then finds it taken, lists the commands and
//! runs none without a ticket. A second run parks the
//! holder, wakes it with the row (its ticket moves to the woken run) and
//! answers Don't Run. A query starting with "plan" runs a plan (`then`):
//! asked once before its first step, or declined and nothing runs. A query
//! starting with "trust" runs the development load's commands with "Don't
//! ask again" (step 4) and then Ask Again Before Plugin Commands. The
//! fixtures speak the wire
//! directly (their own process, `VESTRA_SHARED=0`), so the test does not
//! wait for the SDK release that wraps these calls. Built-ins from a
//! `builtin.json` schema 3 under `VESTRA_BUILTIN_PLUGINS`, the development
//! runner, Node 26 on `PATH` (or `VESTRA_NODE`) and libmpv. Ignored by
//! default; `npm run ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

const ID: &str = "vestra.fixture-lead";
const THIRD: &str = "zysilm.fixture-third";

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

/// A package `id` with `capabilities` and `main.js` in `root`; its
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

/// The raw wire beside the SDK: requests with ids the SDK never uses, the
/// replies and events read from stdin (the runner reads it too).
const WIRE: &str = r#"
import { StringDecoder } from "node:string_decoder";
let next = 1_000_000;
const waiting = new Map();
const handlers = [];
const decoder = new StringDecoder("utf8");
let buffer = "";
process.stdin.on("data", (chunk) => {
  buffer += decoder.write(chunk);
  let at;
  while ((at = buffer.indexOf("\n")) >= 0) {
    const line = buffer.slice(0, at);
    buffer = buffer.slice(at + 1);
    let message;
    try { message = JSON.parse(line); } catch { continue; }
    if (typeof message.id === "number" && waiting.has(message.id)) {
      waiting.get(message.id)(message);
      waiting.delete(message.id);
    } else if (message.event) {
      for (const handler of handlers) handler(message);
    }
  }
});
const call = (method, params) => {
  const id = next++;
  process.stdout.write(JSON.stringify({ id, method, params }) + "\n");
  return new Promise((done) => waiting.set(id, done));
};
const said = (reply) => reply.error ? `error ${reply.error.code} (${reply.error.message})` : JSON.stringify(reply.result);
const log = (...args) => console.error(...args);
"#;

/// The built-in fixture: sets the lead, and on its event runs commands.
fn fixture() -> String {
    format!(
        r#"{WIRE}
export default async function activate(plugin) {{
  plugin.commands.add({{ id: "hello", path: ["Tools", "Hello"], run: () => log("FIX hello ran") }});
  log("FIX set", said(await call("palette.lead.set", {{ title: "Ask Fixture", icon: "ai",
    placeholder: "Type a command or ask", note: "Ready" }})));
  log("FIX forged", said(await call("commands.run", {{ id: "{ID}/hello", ticket: "forged" }})));
  log("FIX unticketed", said(await call("commands.run", {{ id: "{ID}/hello" }})));
  log("FIX register wipe", said(await call("commands.register", {{ id: "wipe", revision: 1,
    declaration: {{ title: "Wipe", path: ["Tools", "Wipe"], destructive: true }},
    state: {{ enabled: true, busy: false }} }})));
  handlers.push(async (message) => {{
    if (message.event === "commands.execute" && message.data.id === "wipe") log("FIX wipe ran");
    if (message.event !== "palette.lead") return;
    const {{ query, ticket, keep, status }} = message.data;
    log(`FIX lead query=${{query}} keep=${{keep}} status=${{status}} ticket=${{ticket.length}}`);
    const run = (id, more = {{}}) => call("commands.run", {{ id, ticket, ...more }});
    const dangerous = async () => {{
      const list = (await call("commands.list", {{}})).result;
      const row = (id) => list.commands.find((c) => c.id === id);
      return JSON.stringify([row("{THIRD}/export")?.dangerous, row("{THIRD}/purge")?.dangerous,
        row("{THIRD}/export")?.builtin]);
    }};
    const pause = () => new Promise((done) => setTimeout(done, 300));
    if (query.startsWith("trust")) {{
      // Don't ask again (H4): the development load's plain command asks
      // once and is remembered; its destructive one still asks; the
      // palette command Ask Again Before Plugin Commands forgets it.
      log("FIX trust list before", await dangerous());
      log("FIX trust export", said(await run("{THIRD}/export")));
      await pause();
      log("FIX trust list after", await dangerous());
      log("FIX trust export again", said(await run("{THIRD}/export")));
      await pause();
      log("FIX trust purge", said(await run("{THIRD}/purge")));
      await pause();
      log("FIX trust ask again", said(await run("host/ask-again")));
      await pause();
      log("FIX trust list reset", await dangerous());
      log("FIX trust done", said(await call("palette.lead.done", {{ ticket }})));
      return;
    }}
    if (query.startsWith("plan")) {{
      // A plan (`then`): asked once before the first step; a command
      // outside the approved plan asks on its own.
      log("FIX plan bad then", said(await run("{ID}/hello", {{ then: "{ID}/wipe" }})));
      log("FIX plan bad id", said(await run("{ID}/hello", {{ then: [7] }})));
      log("FIX plan long", said(await run("{ID}/hello", {{ then: Array(8).fill("{ID}/hello") }})));
      log("FIX plan unknown", said(await run("{ID}/hello", {{ then: ["nobody/none"] }})));
      const first = await run("{ID}/hello", {{ step: 1, of: 3, then: ["{ID}/wipe", "{ID}/hello"] }});
      log("FIX plan first", said(first));
      if (first.result?.ran) {{
        log("FIX plan second", said(await run("{ID}/wipe", {{ step: 2, of: 3 }})));
        log("FIX plan third", said(await run("{ID}/hello", {{ step: 3, of: 3 }})));
        log("FIX plan again", said(await run("{ID}/wipe")));
      }}
      log("FIX plan done", said(await call("palette.lead.done", {{ ticket }})));
      return;
    }}
    log("FIX run hello", said(await run("{ID}/hello", {{ step: 1, of: 2 }})));
    log("FIX run wipe", said(await run("{ID}/wipe")));
    log("FIX run missing", said(await run("nobody/none")));
    log("FIX run palette", said(await run("host/palette")));
    const burst = [];
    for (let i = 0; i < 3; i++) burst.push(said(await run("{ID}/hello")));
    log("FIX burst", burst.join(" | "));
    const list = (await call("commands.list", {{}})).result;
    const row = (id) => list.commands.find((c) => c.id === id);
    log("FIX list", JSON.stringify({{ revision: typeof list.revision,
      hello: [row("{ID}/hello")?.dangerous, row("{ID}/hello")?.builtin, row("{ID}/hello")?.title],
      wipe: row("{ID}/wipe")?.dangerous, quit: row("host/quit")?.dangerous,
      keys: Array.isArray(row("host/play-pause")?.keys) }}));
    log("FIX bad state", said(await call("palette.lead.set", {{ title: "Ask Fixture", icon: "ai",
      status: {{ text: "x", state: "stopped" }} }})));
    log("FIX end", said(await call("palette.lead.set", {{ title: "Ask Fixture", icon: "ai",
      status: {{ text: "Done · Hello", state: "done" }} }})));
    log("FIX end again", said(await call("palette.lead.set", {{ title: "Ask Fixture", icon: "ai",
      placeholder: "Type a command or ask", status: {{ text: "Done · Hello", state: "done" }} }})));
    log("FIX done", said(await call("palette.lead.done", {{ ticket }})));
    log("FIX after done", said(await run("{ID}/hello")));
  }});
}}
"#
    )
}

/// The development load: not built-in. Its commands are a plugin's that is
/// not built in: `export` (plain) and `purge` (destructive). It asks for
/// the row at once (it may start before the built-in fixture's first run,
/// and then holds the row until the built-in sets it) and again 1.5 s
/// later, when the built-in holds it; it lists the commands and tries to
/// run one without a ticket.
fn third() -> String {
    format!(
        r#"{WIRE}
export default async function activate(plugin) {{
  handlers.push((message) => {{
    if (message.event === "commands.execute") log(`THIRD ${{message.data.id}} ran`);
  }});
  for (const [id, destructive] of [["export", false], ["purge", true]]) {{
    log(`THIRD register ${{id}}`, said(await call("commands.register", {{ id, revision: 1,
      declaration: {{ title: id, path: ["Tools", id], destructive }},
      state: {{ enabled: true, busy: false }} }})));
  }}
  log("THIRD early", said(await call("palette.lead.set", {{ title: "Mine", icon: "ai" }})));
  await new Promise((done) => setTimeout(done, 1500));
  log("THIRD set", said(await call("palette.lead.set", {{ title: "Mine", icon: "ai" }})));
  log("THIRD list", typeof (await call("commands.list", {{}})).result?.revision);
  log("THIRD run", said(await call("commands.run", {{ id: "{THIRD}/export" }})));
}}
"#
    )
}

/// Every line of `log` that starts with `prefix`, joined.
fn lines(log: &str, prefix: &str) -> String {
    log.lines()
        .filter(|line| line.contains(prefix))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs the host headless for `seconds` with the built-in fixture and the
/// development load, and `env`; its stderr.
fn run(seconds: u32, env: &[(&str, &str)]) -> String {
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    std::fs::create_dir(&plugins).unwrap();
    let entry = package(&plugins, ID, &["plugins"], fixture());
    std::fs::write(
        plugins.join("builtin.json"),
        serde_json::json!({"schema":3,"plugins":[entry]}).to_string(),
    )
    .unwrap();
    let development = root.path().join("dev");
    package(&development, THIRD, &["plugins"], third());
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    command
        .args([
            "--headless",
            "--silent",
            "--seconds",
            &seconds.to_string(),
            "--plugin",
        ])
        .arg(development.join(THIRD))
        .env("VESTRA_BUILTIN_PLUGINS", &plugins)
        .env("VESTRA_PACKAGE_STORE", root.path().join("store"))
        .env("VESTRA_PLUGIN_DATA", root.path().join("data"))
        .env("VESTRA_SHARED", "0")
        .env_remove("VESTRA_TEST_CONFIRM")
        .env_remove("VESTRA_IDLE_MS")
        .env_remove("VESTRA_INSTANCE_DIR");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    log
}

/// `text` is in the fixture's lines of `log`.
fn expect(log: &str, text: &str) {
    let fix = lines(log, "FIX ");
    assert!(
        fix.contains(text),
        "{text:?} in:\n{fix}\n\nfull log:\n{log}"
    );
}

#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_built_in_lead_gets_the_query_and_runs_commands_with_its_ticket() {
    let logs = tempfile::tempdir().unwrap();
    let host_log = logs.path().join("host.log");
    let log = run(
        8,
        &[
            ("VESTRA_TEST_CONFIRM", "run"),
            ("VESTRA_TEST_EXECUTE", "lead:make it faster@4000"),
            ("VESTRA_LOG_FILE", host_log.to_str().unwrap()),
        ],
    );
    let expect = |text: &str| expect(&log, text);
    // Set while no one else holds it; no run without a live ticket.
    expect("FIX set null");
    expect("FIX forged error ticket");
    expect("FIX unticketed error ticket");
    expect("FIX register wipe {\"revision\"");
    // Activated with the typed text and a ticket.
    expect("FIX lead query=make it faster keep=false status=false ticket=32");
    // Its own command runs, named in the status strip (it holds the lead).
    expect("FIX run hello {\"ran\":true}");
    expect("FIX hello ran");
    assert!(
        log.contains(&format!(
            "commands.run: {ID} ran {ID}/hello (strip: 1/2 · Tools: Hello)"
        )),
        "{log}"
    );
    // The host log keeps what ran, by id (an installed build's stderr goes
    // nowhere), never the label or the OSD line, which may name a file.
    let kept = std::fs::read_to_string(&host_log).unwrap();
    assert!(
        kept.contains(&format!("commands.run: {ID} ran {ID}/hello (step 1/2)"))
            && kept.contains(&format!("commands.run: {ID} ran {ID}/wipe")),
        "{kept}"
    );
    assert!(!kept.contains("osd:") && !kept.contains("Hello"), "{kept}");
    // A destructive command: the host asks first (headless: VESTRA_TEST_CONFIRM).
    assert!(
        log.contains(&format!("commands.run: {ID} asks to run {ID}/wipe")),
        "{log}"
    );
    expect("FIX run wipe {\"ran\":true}");
    expect("FIX wipe ran");
    expect("FIX run missing error missing");
    expect("FIX run palette error invalid");
    // Four a second: hello and wipe, two more, then the limit.
    expect("FIX burst {\"ran\":true} | {\"ran\":true} | error limit");
    expect(
        r#"FIX list {"revision":"number","hello":[false,true,"Hello"],"wipe":true,"quit":true,"keys":true}"#,
    );
    // The status state (SDK 1.7.15): an unknown one is refused; an end state
    // goes to the strip once.
    expect("FIX bad state error invalid");
    expect("FIX end null");
    expect("FIX end again null");
    // Once: the second set repeats the same end status (an update).
    assert_eq!(log.matches("lead strip: done").count(), 1, "{log}");
    expect("FIX done null");
    expect("FIX after done error ticket");
    // Not built-in, the same rules: the row is the first holder's (and a
    // built-in takes it from a plugin that is not: whichever started first,
    // the built-in fixture holds it), the command table is open with
    // `plugins`, a run needs the lead's ticket.
    let third = lines(&log, "THIRD ");
    if third.contains("THIRD early null") {
        assert!(
            log.contains(&format!("palette lead: {ID} takes the row from {THIRD}")),
            "{log}"
        );
    }
    assert!(!log.contains("FIX set error"), "{log}");
    assert!(third.contains("THIRD set error conflict"), "{third}\n{log}");
    assert!(third.contains("THIRD list number"), "{third}\n{log}");
    assert!(third.contains("THIRD run error ticket"), "{third}\n{log}");
}

/// The same end state for two requests in a row shows both times: a new
/// ticket forgets the last end state shown (`Lead::ended`), while the same
/// end status sent again within one request shows once.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn the_same_end_state_shows_again_for_the_next_request() {
    let log = run(
        10,
        &[
            ("VESTRA_TEST_CONFIRM", "run"),
            ("VESTRA_TEST_EXECUTE", "lead:first@4000,lead:second@6500"),
        ],
    );
    expect(
        &log,
        "FIX lead query=first keep=false status=false ticket=32",
    );
    expect(
        &log,
        "FIX lead query=second keep=false status=false ticket=32",
    );
    // Each request sets it twice (the second an update of other fields).
    assert_eq!(lines(&log, "FIX end").lines().count(), 4, "{log}");
    assert_eq!(log.matches("lead strip: done").count(), 2, "{log}");
}

/// The holder is parked while idle (its lead recorded); activating the row
/// wakes it, the ticket minted for the parked owner moves to the woken run
/// (`rekey`) and works there; the user answers Don't Run to the destructive
/// command (headless without `VESTRA_TEST_CONFIRM`), which does not run.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_parked_holder_wakes_with_a_working_ticket_and_dont_run_declines() {
    let log = run(
        12,
        &[
            ("VESTRA_IDLE_MS", "500"),
            ("VESTRA_TEST_EXECUTE", "lead:faster please@7000"),
        ],
    );
    let parked = log
        .find(&format!("lazy: {ID} parked"))
        .unwrap_or_else(|| panic!("parked: {log}"));
    let activated = log
        .find("VESTRA_TEST_EXECUTE lead:faster please")
        .unwrap_or_else(|| panic!("activated: {log}"));
    let woken = log
        .find(&format!("lazy: {ID} woken by palette.lead"))
        .unwrap_or_else(|| panic!("woken: {log}"));
    let query = log
        .find("FIX lead query=faster please")
        .unwrap_or_else(|| panic!("event: {log}"));
    assert!(
        parked < activated && activated < woken && woken < query,
        "{log}"
    );
    let expect = |text: &str| expect(&log, text);
    expect("FIX run hello {\"ran\":true}");
    expect("FIX run wipe {\"declined\":true,\"ran\":false}");
    assert!(!lines(&log, "FIX ").contains("FIX wipe ran"), "{log}");
    expect("FIX done null");
    expect("FIX after done error ticket");
}

/// A plan (`commands.run` `then`, Ask Vestra UX step 3): bad `then` values
/// and an unknown id are refused before anything runs or asks; the host
/// asks once, before the first step, for the whole plan (headless:
/// `VESTRA_TEST_CONFIRM=always` answers Run, as `run` does); the planned
/// destructive step then runs without asking; once used, it asks again.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_plan_is_asked_once_before_its_first_step() {
    let log = run(
        8,
        &[
            ("VESTRA_TEST_CONFIRM", "always"),
            ("VESTRA_TEST_EXECUTE", "lead:plan it@4000"),
        ],
    );
    let expect = |text: &str| expect(&log, text);
    expect("FIX lead query=plan it");
    expect("FIX plan bad then error invalid");
    expect("FIX plan bad id error invalid");
    expect("FIX plan long error invalid");
    expect("FIX plan unknown error missing");
    expect("FIX plan first {\"ran\":true}");
    assert!(
        log.contains(&format!(
            "commands.run: {ID} asks to run {ID}/hello, {ID}/wipe, {ID}/hello (headless: true)"
        )),
        "{log}"
    );
    expect("FIX plan second {\"ran\":true}");
    expect("FIX plan third {\"ran\":true}");
    expect("FIX plan again {\"ran\":true}");
    expect("FIX plan done null");
    // Asked twice in all: the plan, then the wipe outside it.
    assert_eq!(log.matches("asks to run").count(), 2, "{log}");
    assert_eq!(
        log.matches(&format!("asks to run {ID}/wipe (headless: true)"))
            .count(),
        1,
        "{log}"
    );
    assert_eq!(lines(&log, "FIX hello ran").lines().count(), 2, "{log}");
    assert_eq!(lines(&log, "FIX wipe ran").lines().count(), 2, "{log}");
}

/// "Don't ask again" (Ask Vestra UX step 4, H4; headless:
/// `VESTRA_TEST_CONFIRM=always` answers Run with it checked): a plain
/// command of a plugin that is not built in asks once, then neither asks
/// nor lists as `dangerous` for this caller; that plugin's destructive
/// command still asks (no checkbox: nothing remembered); the palette
/// command Ask Again Before Plugin Commands, run as any command, forgets
/// it (its OSD line, the host log).
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn dont_ask_again_trusts_a_plugins_plain_commands_until_ask_again() {
    let logs = tempfile::tempdir().unwrap();
    let host_log = logs.path().join("host.log");
    let log = run(
        9,
        &[
            ("VESTRA_TEST_CONFIRM", "always"),
            ("VESTRA_TEST_EXECUTE", "lead:trust it@4000"),
            ("VESTRA_LOG_FILE", host_log.to_str().unwrap()),
        ],
    );
    let expect = |text: &str| expect(&log, text);
    expect("FIX trust list before [true,true,false]");
    expect("FIX trust export {\"ran\":true}");
    assert!(
        log.contains(&format!(
            "commands.run: {ID} asks to run {THIRD}/export (headless: true, remember)"
        )),
        "{log}"
    );
    expect("FIX trust list after [false,true,false]");
    expect("FIX trust export again {\"ran\":true}");
    expect("FIX trust purge {\"ran\":true}");
    // The destructive one asked, with nothing to remember.
    assert!(
        log.contains(&format!(
            "commands.run: {ID} asks to run {THIRD}/purge (headless: true)"
        )),
        "{log}"
    );
    // Asked twice in all: the first export and the purge.
    assert_eq!(log.matches("asks to run").count(), 2, "{log}");
    assert_eq!(lines(&log, "THIRD export ran").lines().count(), 2, "{log}");
    assert_eq!(lines(&log, "THIRD purge ran").lines().count(), 1, "{log}");
    expect("FIX trust ask again {\"ran\":true}");
    expect("FIX trust list reset [true,true,false]");
    expect("FIX trust done null");
    let kept = std::fs::read_to_string(&host_log).unwrap();
    assert!(
        kept.contains(&format!(
            "commands.run: {ID} no longer asks before the commands of {THIRD}"
        )) && kept.contains("commands.run: plugin commands ask again"),
        "{kept}"
    );
}

/// Don't Run on a plan's sheet: nothing of the plan runs, not even its
/// first step, which does not ask by itself.
#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn dont_run_on_a_plan_runs_nothing() {
    let log = run(8, &[("VESTRA_TEST_EXECUTE", "lead:plan it@4000")]);
    expect(&log, "FIX plan first {\"declined\":true,\"ran\":false}");
    expect(&log, "FIX plan done null");
    assert!(
        log.contains(&format!(
            "commands.run: {ID} asks to run {ID}/hello, {ID}/wipe, {ID}/hello (headless: false)"
        )),
        "{log}"
    );
    let fix = lines(&log, "FIX ");
    assert!(
        !fix.contains("FIX hello ran") && !fix.contains("FIX wipe ran"),
        "{log}"
    );
}
