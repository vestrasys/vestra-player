//! The mpv pass-through with the real host, headless (doc/architecture/
//! README.md §4.9): a built-in fixture that declares `mpv` writes, reads
//! and observes an mpv property, runs commands, adds its own labelled
//! filter and meets the closed refusal rules; a development load (not
//! built-in, declaring nothing) uses the same methods (player 0.9.0;
//! before, it was refused every method). The fixtures speak the wire directly
//! (their own process, `VESTRA_SHARED=0`). Built-ins from a `builtin.json`
//! schema 3 under `VESTRA_BUILTIN_PLUGINS`, the development runner, Node 26
//! on `PATH` (or `VESTRA_NODE`) and libmpv. Ignored by default; `npm run
//! ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;
#[path = "explicit/mod.rs"]
mod explicit;
mod isolation;

const ID: &str = "vestra.fixture-mpv";
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

/// A package `id` with `main.js` (and the calls `capabilities` stand
/// for, `explicit`) in `root`; its `builtin.json` entry.
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

/// The built-in fixture: declares `mpv` and uses the pass-through.
fn fixture() -> String {
    format!(
        r#"{WIRE}
export default async function activate(plugin) {{
  handlers.push((message) => {{
    if (message.event === "mpv.property") log("FIX event", JSON.stringify(message.data));
  }});
  const command = (...args) => call("mpv.command", {{ args }});
  log("FIX set", said(await call("mpv.set", {{ name: "brightness", value: 7 }})));
  log("FIX get", said(await call("mpv.get", {{ name: "brightness" }})));
  log("FIX observe", said(await call("mpv.observe", {{ name: "brightness" }})));
  log("FIX add", said(await command("add", "brightness", 2)));
  log("FIX quit", said(await command("quit")));
  log("FIX pause", said(await call("mpv.set", {{ name: "pause", value: true }})));
  log("FIX shape", said(await call("mpv.command", {{ args: "seek 1" }})));
  log("FIX missing", said(await call("mpv.get", {{ name: "no-such-property" }})));
  log("FIX filter", said(await command("vf", "add", "@fix:lavfi=[null]")));
  log("FIX host filter", said(await command("vf", "remove", "@vestra-orientation")));
  log("FIX hidden", said(await command("vf", "add", "@two:lavfi=[null],hflip")));
  const chain = await call("mpv.get", {{ name: "vf" }});
  log("FIX chain", JSON.stringify(chain.result.value.map((filter) => filter.label)));
  await new Promise((done) => setTimeout(done, 1500));
  log("FIX later", said(await call("mpv.get", {{ name: "brightness" }})));
}}
"#
    )
}

/// The development load: a plugin that is not built-in, which needs no
/// declaration for the pass-through either (player 0.9.0). It works on
/// `contrast`, so that the fixture's `brightness` values stay its own now
/// that its calls succeed.
fn third() -> String {
    format!(
        r#"{WIRE}
export default async function activate(plugin) {{
  log("THIRD get", said(await call("mpv.get", {{ name: "contrast" }})));
  log("THIRD command", said(await call("mpv.command", {{ args: ["add", "contrast", 1] }})));
  log("THIRD observe", said(await call("mpv.observe", {{ name: "contrast" }})));
}}
"#
    )
}

/// Runs the host headless for `seconds` with the built-in fixture and the
/// development load; its stderr.
fn run(seconds: u32) -> String {
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    std::fs::create_dir(&plugins).unwrap();
    let entry = package(&plugins, ID, &["mpv"], fixture());
    std::fs::write(
        plugins.join("builtin.json"),
        serde_json::json!({"schema":3,"plugins":[entry]}).to_string(),
    )
    .unwrap();
    let development = root.path().join("dev");
    package(&development, THIRD, &[], third());
    let output = isolation::isolate(
        &mut Command::new(env!("CARGO_BIN_EXE_vestra-host")),
        root.path(),
    )
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
    .env_remove("VESTRA_IDLE_MS")
    .env_remove("VESTRA_TEST_EXECUTE")
    .env_remove("VESTRA_TEST_PARK")
    .env_remove("VESTRA_INSTANCE_DIR")
    .output()
    .unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    log
}

#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn a_plugin_with_the_mpv_capability_drives_mpv_within_the_rules() {
    let log = run(6);
    let fix = log
        .lines()
        .filter(|line| line.contains("FIX "))
        .collect::<Vec<_>>()
        .join("\n");
    let expect = |text: &str| {
        assert!(
            fix.contains(text),
            "{text:?} in:\n{fix}\n\nfull log:\n{log}"
        )
    };
    // Properties and commands reach the player's mpv.
    expect("FIX set null");
    expect(r#"FIX get {"value":7}"#);
    expect("FIX observe null");
    expect(r#"FIX add {"result":null}"#);
    expect(r#"FIX later {"value":9}"#);
    // The observer: the value at once, then the change, to it alone.
    expect(r#"FIX event {"name":"brightness","value":7}"#);
    expect(r#"FIX event {"name":"brightness","value":9}"#);
    assert!(!log.contains("THIRD event"), "{log}");
    // The closed rules, the shape and mpv's own errors.
    expect(r#"FIX quit error refused (mpv command "quit" is refused: it ends the player)"#);
    expect(r#"FIX pause error refused (mpv property "pause" is refused: "#);
    expect("FIX shape error invalid (a command is an array of 1–32 elements, its name first)");
    expect("FIX missing error backend (mpv: property not found)");
    // Its own labelled filter goes in after the player's; the player's own
    // and a second filter behind the label are refused, the chain kept.
    expect(r#"FIX filter {"result":null}"#);
    expect(r#"FIX host filter error refused (mpv command "vf" is refused: "#);
    expect(r#"FIX hidden error refused (mpv command "vf" is refused: "#);
    expect(r#"FIX chain ["vestra-orientation","vestra-orientation-metadata","fix"]"#);
    // A filter and an observer keep it running: never in standby.
    assert!(!log.contains(&format!("lazy: {ID} parked")), "{log}");
    // Player 0.9.0: a plugin that is not built-in uses it without a
    // declaration (before player 0.9.0 each call was refused without one).
    for expected in [
        r#"THIRD get {"value":0}"#,
        r#"THIRD command {"result":null}"#,
        "THIRD observe null",
    ] {
        assert!(log.contains(expected), "{expected}\n{log}");
    }
}
