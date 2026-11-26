//! The second batch of events (release B): each is sent by the running player and wakes a plugin
//! in standby. The host offscreen (`VESTRA_TEST_OFFSCREEN=1`, so the
//! interface's facts flow), the shared runner, a short idle time; one
//! listener per event, each a built-in that listens to that event alone
//! (a listener of a wake event keeps nothing running, so each is parked),
//! and a driver that stays up and makes the facts change: it mounts a
//! bottom panel and hides it (`layout.changed`), opens a video with audio
//! and a subtitle track and then a second one (`video.reconfigured`,
//! `audio.reconfigured`, `audio.tracks.changed`, `subtitles.changed` for
//! each source), and turns fullscreen on (`window.changed`). The player
//! runs twice on the same profile: the second run's `app.started` names
//! the first run's version.
//!
//! `audio.devices.changed` needs an audio device to come or go; the
//! offscreen run has none (its decision is a unit test,
//! `source_backend.rs`). The listeners use the wire (`plugin.wire.on`), so
//! they log the data in the player's wire shape.
//!
//! Needs Node 26 on `PATH` (or `VESTRA_NODE`), the development runner,
//! libmpv and ffmpeg (`VESTRA_FFMPEG`). Ignored by default; `npm run
//! ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;
mod isolation;

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

/// A bottom panel (`ui.mount`).
const PANEL: &str = "export global Bridge {\n    in property <bool> tabbed;\n}\n\
     export component List inherits Rectangle {\n    in property <string> label;\n\
     background: #1a1821;\n    Text { text: label; color: white; }\n}\n";

/// A built-in package `id` under `root` with `main` as its `main.js`; its
/// `builtin.json` entry.
fn package(root: &Path, id: &str, main: String) -> Value {
    let files = [
        (
            "manifest.json",
            json!({"id":id,"version":"1.0.0","apiVersion":"vestra/1","entry":"main.js"})
                .to_string(),
        ),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        ("main.js", main),
        ("ui/list.slint", PANEL.to_owned()),
    ];
    let directory = root.join(id);
    for (name, text) in &files {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    json!({"id":id,"version":"1.0.0","directory":id,"digest":digest(&files)})
}

/// A listener of `event` alone, through the wire (`plugin.wire.on`
/// subscribes an event the SDK knows, and hands the listener the wire
/// shape): it logs `FACT|<id>|<event>|<data>` for each one.
fn listener(id: &str, event: &str) -> String {
    format!(
        "export default async function activate(plugin) {{\n\
         console.error(`{id} start`);\n\
         plugin.wire.on(\"{event}\", (data) => console.error(`FACT|{id}|{event}|${{JSON.stringify(data)}}`));\n}}\n"
    )
}

/// Runs the player offscreen for `seconds` under `root`; its stderr.
fn run(root: &Path, seconds: u32) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_vestra-host"));
    isolation::isolate(&mut command, root);
    command
        .args(["--silent", "--seconds", &seconds.to_string()])
        .env("VESTRA_TEST_OFFSCREEN", "1")
        .env("VESTRA_BUILTIN_PLUGINS", root.join("plugins"))
        .env("VESTRA_INSTANCE_DIR", root.join("instance"))
        .env("VESTRA_LANGUAGE", "en")
        .env("VESTRA_SHARED", "1")
        .env("VESTRA_IDLE_MS", "500");
    for key in [
        "VESTRA_TEST_EXECUTE",
        "VESTRA_TEST_PARK",
        "VESTRA_TEST_WAKE",
        "VESTRA_TEST_UI_STALL",
        "VESTRA_FRAME_LOG",
    ] {
        command.env_remove(key);
    }
    let output = command.output().unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    if let Some(dir) = std::env::var_os("VESTRA_TEST_LOG_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::write(dir.join(format!("fact-events-{seconds}.log")), &log);
    }
    log
}

/// What `id` heard of `event`, in order, as JSON.
fn heard(log: &str, id: &str, event: &str) -> Vec<Value> {
    let marker = format!("FACT|{id}|{event}|");
    log.lines()
        .filter_map(|line| {
            let at = line.find(&marker)?;
            serde_json::from_str(&line[at + marker.len()..]).ok()
        })
        .collect()
}

/// A media file of `seconds` with a 320x240 video at 25 fps, a sine tone
/// and one SRT subtitle track; its path as a JSON string.
fn media(dir: &Path, name: &str, seconds: u32) -> String {
    let ffmpeg = std::env::var("VESTRA_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let srt = dir.join(format!("{name}.srt"));
    std::fs::write(&srt, "1\n00:00:00,500 --> 00:00:05,000\nHello\n\n").unwrap();
    let path = dir.join(format!("{name}.mkv"));
    let made = Command::new(&ffmpeg)
        .args(["-v", "error", "-f", "lavfi", "-i"])
        .arg(format!("testsrc=duration={seconds}:size=320x240:rate=25"))
        .args(["-f", "lavfi", "-i"])
        .arg(format!("sine=duration={seconds}:sample_rate=48000"))
        .arg("-i")
        .arg(&srt)
        .args(["-map", "0", "-map", "1", "-map", "2"])
        .args([
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-c:s", "srt", "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(made.success(), "ffmpeg made {}", path.display());
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

const EVENTS: [(&str, &str); 7] = [
    ("vestra.fixture-video", "video.reconfigured"),
    ("vestra.fixture-audio", "audio.reconfigured"),
    ("vestra.fixture-tracks", "audio.tracks.changed"),
    ("vestra.fixture-subtitles", "subtitles.changed"),
    ("vestra.fixture-window", "window.changed"),
    ("vestra.fixture-layout", "layout.changed"),
    ("vestra.fixture-started", "app.started"),
];
const DRIVER: &str = "vestra.fixture-driver";

#[test]
#[ignore = "needs Node 26, libmpv and ffmpeg: run with --include-ignored (ci:local)"]
fn fact_events_are_sent_and_wake_their_listeners() {
    let files = tempfile::tempdir().unwrap();
    let (first, second) = (media(files.path(), "a", 30), media(files.path(), "b", 30));
    let root = tempfile::tempdir().unwrap();
    let plugins = root.path().join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    let mut entries = EVENTS
        .iter()
        .map(|(id, event)| package(&plugins, id, listener(id, event)))
        .collect::<Vec<_>>();
    // The driver: a state listener keeps it running, so its timers hold.
    entries.push(package(
        &plugins,
        DRIVER,
        format!(
            "export default function activate(plugin) {{\n\
             plugin.player.on(\"source.state\", () => {{}});\n\
             const step = (at, name, act) => setTimeout(() => Promise.resolve().then(act).then(\n\
             () => console.error(`{DRIVER} ${{name}}`),\n\
             (error) => console.error(`{DRIVER} ${{name}} failed ${{error?.code ?? error}}`)), at);\n\
             step(3000, \"mount\", () => plugin.wire.request(\"ui.mount\", {{ slot: \"bottom\", file: \"ui/list.slint\",\n\
             component: \"List\", global: \"Bridge\", title: \"Driver\", properties: {{}} }}));\n\
             step(4000, \"open a\", () => plugin.player.open({first}));\n\
             step(8000, \"hide\", () => plugin.wire.request(\"layout.set\", {{ slot: \"bottom\", visible: false }}));\n\
             step(9000, \"fullscreen\", () => plugin.wire.request(\"host.run\", {{ action: \"fullscreen\" }}));\n\
             step(11000, \"open b\", () => plugin.player.open({second}));\n}}\n"
        ),
    ));
    std::fs::write(
        plugins.join("builtin.json"),
        json!({"schema":3,"plugins":entries}).to_string(),
    )
    .unwrap();

    let log = run(root.path(), 16);
    for step in ["mount", "open a", "hide", "fullscreen", "open b"] {
        assert!(
            log.lines()
                .any(|line| line.trim_end().ends_with(&format!("{DRIVER} {step}"))),
            "the driver's {step}: {log}"
        );
    }
    // Every listener but `app.started`'s (the first run's comes at the end
    // of start-up, which the listener may still be up for) is parked, then
    // woken by its event.
    for (id, event) in EVENTS.iter().filter(|(_, event)| *event != "app.started") {
        let parked = log
            .find(&format!("lazy: {id} parked"))
            .unwrap_or_else(|| panic!("{id} parked: {log}"));
        let woken = log
            .find(&format!("lazy: {id} woken by {event}"))
            .unwrap_or_else(|| panic!("{id} woken by {event}: {log}"));
        assert!(parked < woken, "{id}: {log}");
        assert!(
            !heard(&log, id, event).is_empty(),
            "{id} heard {event}: {log}"
        );
    }
    // The facts of each source, for both files (source ids from 1).
    let video = heard(&log, "vestra.fixture-video", "video.reconfigured");
    let sources = |facts: &[Value]| {
        let mut ids = facts
            .iter()
            .filter_map(|f| f["sourceId"].as_u64())
            .collect::<Vec<_>>();
        ids.dedup();
        ids
    };
    assert_eq!(sources(&video), [1, 2], "{video:?}");
    let first_video = &video[0];
    assert_eq!(first_video["width"], 320, "{first_video}");
    assert_eq!(first_video["height"], 240, "{first_video}");
    assert_eq!(first_video["hdr"], false, "{first_video}");
    assert!(
        (first_video["fps"].as_f64().unwrap_or(0.) - 25.).abs() < 0.01,
        "{first_video}"
    );
    let audio = heard(&log, "vestra.fixture-audio", "audio.reconfigured");
    assert_eq!(audio[0]["sampleRate"], 48000, "{audio:?}");
    let tracks = heard(&log, "vestra.fixture-tracks", "audio.tracks.changed");
    assert_eq!(sources(&tracks), [1, 2], "{tracks:?}");
    assert_eq!(tracks[0]["audioTracks"].as_array().map(Vec::len), Some(1));
    let subtitles = heard(&log, "vestra.fixture-subtitles", "subtitles.changed");
    let generations = subtitles
        .iter()
        .filter_map(|s| s["state"]["source"]["generation"].as_u64())
        .collect::<Vec<_>>();
    assert!(
        generations.contains(&1) && generations.contains(&2),
        "{subtitles:?}"
    );
    assert_eq!(
        subtitles[0]["state"]["tracks"].as_array().map(Vec::len),
        Some(1),
        "the file's one subtitle track: {subtitles:?}"
    );
    // The panel came, then was hidden: the last layout has it hidden.
    let layouts = heard(&log, "vestra.fixture-layout", "layout.changed");
    assert!(
        layouts
            .iter()
            .any(|l| l["bottom"]["preferred"] == false && l["bottom"]["effective"] == false),
        "{layouts:?}"
    );
    // The panel took room from the video and gave it back: one event per
    // size (sent once the size settled, not at each layout step); the
    // fullscreen flip once, at once.
    let windows = heard(&log, "vestra.fixture-window", "window.changed");
    let sizes = windows
        .iter()
        .filter(|w| w["fullscreen"] == false)
        .map(|w| w["videoArea"]["height"].clone())
        .collect::<Vec<_>>();
    let mut distinct = sizes.clone();
    distinct.dedup();
    assert_eq!(sizes, distinct, "one event per settled size: {windows:?}");
    assert!(
        sizes.len() >= 2,
        "the panel changed the video area: {windows:?}"
    );
    assert_eq!(
        windows.iter().filter(|w| w["fullscreen"] == true).count(),
        1,
        "{windows:?}"
    );
    // The first run's start, without a version before it.
    let started = heard(&log, "vestra.fixture-started", "app.started");
    assert_eq!(started.len(), 1, "{log}");
    assert_eq!(started[0]["previous"], Value::Null, "{started:?}");
    let version = started[0]["version"].as_str().unwrap().to_owned();

    // The second run: every listener starts in standby (its recording),
    // and `app.started` wakes its listener, naming the first run's version.
    let again = run(root.path(), 6);
    let id = "vestra.fixture-started";
    assert!(
        again.contains(&format!("lazy: {id} woken by app.started")),
        "{again}"
    );
    let started = heard(&again, id, "app.started");
    assert_eq!(started.len(), 1, "{again}");
    assert_eq!(started[0]["version"], version.as_str());
    assert_eq!(started[0]["previous"], version.as_str());
}
