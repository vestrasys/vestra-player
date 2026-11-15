//! The adjust column detaches into its own window as the sidebar does (user
//! request 2026-11-15), with the windowed host running offscreen
//! (`VESTRA_TEST_OFFSCREEN=1`, `ui::offscreen`: no window on screen). A
//! development plugin mounts an adjust panel, opens the column and detaches
//! it with `layout.set` (`plugin.panels.adjust.setDetached`), and logs
//! `layout.state`'s adjust column after each step. The state file keeps
//! the column detached with its floating window's frame; the next start
//! opens it detached again, in the remembered frame, and docking it again
//! keeps the frame for the next time.
//!
//! Needs Node 26 on `PATH` (or `VESTRA_NODE`), the development runner and
//! libmpv. Ignored by default; `npm run ci:local` runs it
//! (`--include-ignored`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Stdio};

mod isolation;

const PLUGIN: &str = "vestra.fixture-adjust-detach";

/// The development plugin: mounts an adjust panel, opens the column (once
/// the panel is there: `missing` before), runs `steps` (`setDetached`
/// values) and logs the column's state first and after each step:
/// `adjust-detach <tag> <state>`.
fn plugin(directory: &Path, steps: &[bool]) {
    std::fs::create_dir_all(directory.join("ui")).unwrap();
    std::fs::write(
        directory.join("manifest.json"),
        json!({"id":PLUGIN,"version":"1.0.0","apiVersion":"vestra/1","entry":"main.js"})
            .to_string(),
    )
    .unwrap();
    std::fs::write(directory.join("package.json"), r#"{"type":"module"}"#).unwrap();
    std::fs::write(
        directory.join("ui/panel.slint"),
        "export global Bridge { in property <bool> tabbed; }\n\
         export component Panel inherits Rectangle { background: #123; }\n",
    )
    .unwrap();
    std::fs::write(
        directory.join("main.js"),
        format!(
            "export default async function activate(plugin) {{\n\
               plugin.panels.mount(\"adjust\", \"ui/panel.slint\", {{ title: \"Fixture\" }});\n\
               const adjust = plugin.panels.adjust;\n\
               // The SDK reads `layout.state` once in flight: a read asked\n\
               // meanwhile is folded into it, so wait for a value.\n\
               const log = async (tag) => {{\n\
                 for (let tries = 0; adjust.state === null && tries < 40; tries++)\n\
                   await new Promise((done) => setTimeout(done, 50));\n\
                 console.error(`adjust-detach ${{tag}} ${{JSON.stringify(adjust.state)}}`);\n\
               }};\n\
               for (let tries = 0; ; tries++) {{\n\
                 try {{ await adjust.setVisible(true); break; }}\n\
                 catch (error) {{ if (tries > 100) throw error; await new Promise((done) => setTimeout(done, 50)); }}\n\
               }}\n\
               await log(\"start\");\n\
               for (const detached of {steps}) {{\n\
                 await adjust.setDetached(detached);\n\
                 // A tick for the window to open or close, then the state again.\n\
                 await new Promise((done) => setTimeout(done, 300));\n\
                 await adjust.setVisible(true);\n\
                 await log(detached ? \"detached\" : \"docked\");\n\
               }}\n\
               console.error(\"adjust-detach done\");\n\
             }}\n",
            steps = json!(steps),
        ),
    )
    .unwrap();
}

/// Run the player offscreen for `seconds` with the plugin in `development`;
/// its stderr.
fn run(root: &Path, development: &Path, seconds: u32) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    isolation::isolate(&mut command, root);
    command
        .args(["--silent", "--seconds", &seconds.to_string(), "--plugin"])
        .arg(development)
        .env("VESTRA_TEST_OFFSCREEN", "1")
        .env("VESTRA_BUILTIN_PLUGINS", root.join("plugins"))
        .env("VESTRA_LANGUAGE", "en");
    for key in [
        "VESTRA_SHARED",
        "VESTRA_IDLE_MS",
        "VESTRA_TEST_EXECUTE",
        "VESTRA_TEST_PARK",
        "VESTRA_TEST_WAKE",
        "VESTRA_TEST_WIRE_TRACE",
        "VESTRA_TEST_UI_STALL",
        "VESTRA_FRAME_LOG",
    ] {
        command.env_remove(key);
    }
    let output = command
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    log
}

/// The adjust column's state the plugin logged as `tag`.
fn logged(log: &str, tag: &str) -> Value {
    let prefix = format!("adjust-detach {tag} ");
    let line = log
        .lines()
        .find_map(|line| line.find(&prefix).map(|at| &line[at + prefix.len()..]))
        .unwrap_or_else(|| panic!("no `{prefix}` line:\n{log}"));
    serde_json::from_str(line.trim()).unwrap_or_else(|error| panic!("{error}: {line}"))
}

/// The state file's `adjustSlot`.
fn adjust_slot(root: &Path) -> Value {
    let text = std::fs::read_to_string(root.join("ui-state.json")).unwrap();
    serde_json::from_str::<Value>(&text).unwrap()["adjustSlot"].clone()
}

#[test]
#[ignore = "needs Node 26 and libmpv: run with --include-ignored (ci:local)"]
fn the_adjust_column_detaches_is_remembered_and_docks_again() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let plugins = root.join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    std::fs::write(
        plugins.join("builtin.json"),
        json!({"schema":3,"plugins":[]}).to_string(),
    )
    .unwrap();
    let development = root.join("development").join(PLUGIN);

    // First run: docked, then detached into its own window.
    plugin(&development, &[true]);
    let log = run(root, &development, 8);
    assert!(log.contains("adjust-detach done"), "{log}");
    let start = logged(&log, "start");
    assert_eq!(start["detached"], false, "{log}");
    assert_eq!(start["effective"], true, "{log}");
    let detached = logged(&log, "detached");
    assert_eq!(
        detached,
        json!({"preferred":true,"effective":true,"reason":null,"detached":true}),
        "{log}"
    );
    let slot = adjust_slot(root);
    assert_eq!(slot["visible"], true, "{slot}");
    assert_eq!(slot["detached"], true, "{slot}");
    let frame = &slot["floating"];
    assert!(
        frame["width"].as_f64().is_some_and(|w| w >= 280.)
            && frame["height"].as_f64().is_some_and(|h| h >= 250.),
        "the floating window's frame is remembered: {slot}"
    );

    // The user moved and resized it: that frame opens at the next start.
    let mut state: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("ui-state.json")).unwrap())
            .unwrap();
    state["adjustSlot"]["floating"] = json!({"x":120,"y":80,"width":333,"height":444});
    std::fs::write(root.join("ui-state.json"), state.to_string()).unwrap();

    // Second run: it starts detached (as remembered), then docks again.
    plugin(&development, &[false]);
    let log = run(root, &development, 8);
    let start = logged(&log, "start");
    assert_eq!(start["detached"], true, "starts floating: {log}");
    assert_eq!(start["effective"], true, "{log}");
    let docked = logged(&log, "docked");
    assert_eq!(docked["detached"], false, "{log}");
    assert_eq!(docked["effective"], true, "{log}");
    let slot = adjust_slot(root);
    assert!(
        slot.get("detached").is_none(),
        "docked: not written: {slot}"
    );
    assert_eq!(slot["visible"], true, "{slot}");
    // The window opened in the remembered size, which stays for next time.
    assert_eq!(slot["floating"]["width"], 333.0, "{slot}");
    assert_eq!(slot["floating"]["height"], 444.0, "{slot}");
}
