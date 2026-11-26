//! Panel compiles at start-up, with the real built-ins and the windowed
//! host running offscreen (`VESTRA_TEST_OFFSCREEN=1`, `ui::offscreen`: no
//! window on screen). The shell compiles once a run and names no panel;
//! each panel compiles on its own when it is first shown (`ui/panels.rs`;
//! the adjust column's front tabs all at its first opening), which blocks
//! the UI thread for tens to a few hundred milliseconds, and a start with a file argument (a double-click in Explorer) used to play
//! while the start-up panels still compiled: whatever layout was remembered
//! (the adjust column open or closed, a third-party sidebar tab), each
//! panel shown at start-up compiles once, before the file opens, a panel
//! not shown compiles nothing, and opening a file compiles nothing.
//!
//! Each case runs a fresh profile twice: once to record the plugins' first
//! runs (as any player started before), then the start that is counted; one
//! case is a first start. A
//! second file is then opened in the same player by a second launch (handed
//! over, `platform::single_instance`). Needs Node 26 on `PATH`, libmpv,
//! ffmpeg (`VESTRA_FFMPEG`) and the staged built-ins (`VESTRA_TEST_BUILTINS`,
//! else staged with `npm ci` done in src). Ignored by default; `npm run
//! ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
#[path = "explicit/mod.rs"]
mod explicit;
mod isolation;
use std::time::Duration;

/// `package_stage::read_tree`'s digest over a fixture (tests/lazy_activation.rs).
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

const THIRD_PARTY: &str = "aa.fixture-sidebar";

/// A remembered third-party sidebar tab.
#[derive(Clone, Copy, PartialEq, Debug)]
enum ThirdParty {
    None,
    /// Mounts at activation only: parked once idle, its tab replayed as a
    /// placeholder at the next start.
    Parked,
    /// Also listens to the playback state: it stays running.
    Resident,
    /// Resident at the last run, and now its process ends during every
    /// activation (restarted after a crash, with a growing delay).
    Crashing,
}

/// A third-party plugin with a sidebar tab, installed in the library under
/// `store` (its record in `catalog.json`).
fn install_sidebar_plugin(store: &Path, kind: ThirdParty) {
    let (capabilities, listen) = match kind {
        ThirdParty::Resident => (
            ["ui.mount", "events", "playback.read"].as_slice(),
            "plugin.player.on(\"source.state\", () => {});\n",
        ),
        // Its first run (the warm-up) records it resident; from the next
        // start on it exits as soon as its panel is in the shell (each run
        // mounts it, then crashes: the crash loop).
        ThirdParty::Crashing => (
            ["ui.mount", "events", "playback.read"].as_slice(),
            "plugin.player.on(\"source.state\", () => {});\n\
             const mark = join(plugin.dataDirectory, \"ran\");\n\
             crash = existsSync(mark);\n\
             if (!crash) { mkdirSync(plugin.dataDirectory, { recursive: true }); \
             writeFileSync(mark, \"1\"); }\n",
        ),
        _ => (["ui.mount"].as_slice(), ""),
    };
    let manifest = serde_json::json!({"id":THIRD_PARTY,"version":"1.0.0","apiVersion":"vestra/1",
        "entry":"main.js"});
    let files = [
        ("manifest.json", manifest.to_string()),
        ("package.json", r#"{"type":"module"}"#.to_owned()),
        (
            "main.js",
            explicit::main(
                capabilities,
                &format!(
                    "import {{ existsSync, mkdirSync, writeFileSync }} from \"node:fs\";\n\
                 import {{ join }} from \"node:path\";\n\
                 export default function activate(plugin) {{\nlet crash = false;\n{listen}\
                 const panel = plugin.panels.mount(\"sidebar\", \"ui/list.slint\", {{ title: \"Fixture\" }});\n\
                 if (crash) panel.ready.then(() => setTimeout(() => process.exit(7), 20));\n}}\n"
                ),
            ),
        ),
        (
            "ui/list.slint",
            "export global Bridge {\n    in property <bool> tabbed;\n}\n\
             export component List inherits Rectangle {\n    background: #1a1821;\n\
             Text { text: \"fixture\"; color: white; }\n}\n"
                .to_owned(),
        ),
    ];
    let digest = digest(&files);
    let directory = store.join("packages").join(&digest);
    for (name, text) in &files {
        let path = directory.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let catalog = serde_json::json!({"installed":{THIRD_PARTY:{"revision":1,
        "current":{"digest":digest,"manifest":manifest,"directory":directory},
        "previous":null,
        "instance":{"guid":"00000000-0000-4000-8000-000000000001",
            "source":{"kind":"file"},"installedAt":1}}},
        "pending":{},"operations":{},"lastCommit":{}});
    std::fs::create_dir_all(store).unwrap();
    std::fs::write(store.join("catalog.json"), catalog.to_string()).unwrap();
}

/// The staged built-ins (`VESTRA_TEST_BUILTINS`, or staged under `root`).
///
/// Staging packs each plugin in place (`plugins/*/dist`), so two stagings at
/// once break each other (ENOENT, "pack ... failed"): this process stages
/// once, for all its tests (cargo runs them in parallel), into a directory
/// kept until it ends; another process staging at the same time waits for a
/// lock file (one older than ten minutes is taken for a crashed holder's).
fn builtins() -> PathBuf {
    use std::sync::OnceLock;
    if let Some(dir) = std::env::var_os("VESTRA_TEST_BUILTINS") {
        return PathBuf::from(dir);
    }
    static STAGED: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
    STAGED
        .get_or_init(|| {
            let root = tempfile::tempdir().unwrap();
            let staged = root.path().join("plugins");
            let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
            // In the system's temp directory, not the source tree (stagings
            // of other worktrees wait too, which costs only time).
            let _lock = StageLock::take(&std::env::temp_dir().join("vestra-stage-plugins.lock"));
            let status = Command::new("node")
                .arg("scripts/stage-plugins.mjs")
                .arg(&staged)
                .current_dir(&source)
                .status()
                .expect("node on PATH");
            assert!(status.success(), "stage-plugins.mjs failed");
            (root, staged)
        })
        .1
        .clone()
}

/// A lock file held while staging (`builtins`), removed when dropped.
struct StageLock(PathBuf);

impl StageLock {
    fn take(path: &Path) -> Self {
        loop {
            match std::fs::File::create_new(path) {
                Ok(_) => return Self(path.to_owned()),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = std::fs::metadata(path)
                        .and_then(|m| m.modified())
                        .ok()
                        .and_then(|at| at.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(600));
                    if stale {
                        let _ = std::fs::remove_file(path);
                    } else {
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
                Err(error) => panic!("stage lock {}: {error}", path.display()),
            }
        }
    }
}

impl Drop for StageLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A short video clip under `dir`.
fn clip(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let ffmpeg = std::env::var("VESTRA_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let made = Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30:duration=30",
            "-f",
            "lavfi",
            "-i",
            "sine=duration=30",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-shortest",
            "-y",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(made.success());
    path
}

/// One profile: the player's data under `dir`.
struct Profile {
    dir: tempfile::TempDir,
    plugins: PathBuf,
}

impl Profile {
    fn new(plugins: &Path, adjust_open: bool, third_party: ThirdParty) -> Self {
        let dir = tempfile::tempdir().unwrap();
        if third_party != ThirdParty::None {
            install_sidebar_plugin(&dir.path().join("store"), third_party);
        }
        // The Adjustments and Effects column left open (or closed) on the
        // Picture tab at the last run.
        std::fs::write(
            dir.path().join("ui-state.json"),
            serde_json::json!({"version":1,"adjustSlot":{"visible":adjust_open,"width":317,
                "active":"vestra.video","order":[]}})
            .to_string(),
        )
        .unwrap();
        Self {
            dir,
            plugins: plugins.to_owned(),
        }
    }
    fn command(&self, args: &[&std::ffi::OsStr]) -> Command {
        let root = self.dir.path();
        // `VESTRA_TEST_HOST_EXE`: another build of the host, to compare
        // (before and after a change).
        let exe = std::env::var_os("VESTRA_TEST_HOST_EXE").map_or_else(
            || PathBuf::from(env!("CARGO_BIN_EXE_vestra-host")),
            PathBuf::from,
        );
        let mut command = Command::new(exe);
        isolation::isolate(&mut command, root);
        command
            .arg("--silent")
            .args(args)
            .env("VESTRA_TEST_OFFSCREEN", "1")
            .env("VESTRA_BUILTIN_PLUGINS", &self.plugins)
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
            "VESTRA_LOG_FILE",
            "VESTRA_FRAME_LOG",
        ] {
            command.env_remove(key);
        }
        command
    }
    /// Run for `seconds` with `file`; `second`: another file opened by a
    /// second launch `after` the first file opened. The first run's stderr.
    fn run(&self, seconds: u32, file: Option<&Path>, second: Option<(&Path, Duration)>) -> String {
        use std::io::BufRead;
        let seconds = seconds.to_string();
        let mut args = vec![std::ffi::OsStr::new("--seconds"), seconds.as_ref()];
        if let Some(file) = file {
            args.push(file.as_os_str());
        }
        let mut child = self
            .command(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // Its log, line by line: the first open is seen as it happens.
        let stderr = child.stderr.take().unwrap();
        let (opened, first_open) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut log = String::new();
            for line in std::io::BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                if line.contains(" handed over at +") {
                    let _ = opened.send(());
                }
                log.push_str(&line);
                log.push('\n');
            }
            log
        });
        if let Some((file, after)) = second
            && first_open.recv_timeout(Duration::from_secs(30)).is_ok()
        {
            std::thread::sleep(after);
            // `--seconds`: should it not find the player (gone already), it
            // ends on its own.
            let handed = self
                .command(&[
                    std::ffi::OsStr::new("--seconds"),
                    "30".as_ref(),
                    file.as_os_str(),
                ])
                .output()
                .unwrap();
            assert!(
                handed.status.success(),
                "{}",
                String::from_utf8_lossy(&handed.stderr)
            );
        }
        let status = child.wait().unwrap();
        let log = reader.join().unwrap();
        assert!(status.success(), "{log}");
        log
    }
}

/// A compile of a panel's file in the log (`Host::log_compile`).
#[derive(Debug)]
struct Compile {
    at_ms: u64,
    took_ms: f64,
    panels: String,
}

fn compiles(log: &str) -> Vec<Compile> {
    log.lines()
        .filter_map(|line| {
            let rest = line.split("panel: compile ").nth(1)?;
            let panels = rest.split('[').nth(1)?.split(']').next()?.to_owned();
            let took_ms = rest
                .split("] in ")
                .nth(1)?
                .split(" ms")
                .next()?
                .parse()
                .ok()?;
            let at_ms = rest
                .split(" at +")
                .nth(1)?
                .split(" ms")
                .next()?
                .parse()
                .ok()?;
            Some(Compile {
                at_ms,
                took_ms,
                panels,
            })
        })
        .collect()
}

/// When each file was handed to the plugins (`open_files_tick`).
fn opens(log: &str) -> Vec<u64> {
    log.lines()
        .filter_map(|line| {
            let rest = line.split("open: ").nth(1)?;
            rest.split(" handed over at +")
                .nth(1)?
                .split(" ms")
                .next()?
                .parse()
                .ok()
        })
        .collect()
}

fn report(name: &str, log: &str) -> (Vec<Compile>, Vec<u64>) {
    let compiles = compiles(log);
    let opens = opens(log);
    let phases = log
        .lines()
        .filter_map(|line| line.split("source: phase ").nth(1))
        .collect::<Vec<_>>();
    use std::io::Write;
    let mut out = std::io::stderr();
    let _ = writeln!(
        out,
        "== {name}: {} compile(s), opens at {opens:?} ms, phases {phases:?}",
        compiles.len()
    );
    for compile in &compiles {
        // The line is written when the compile ended.
        let began = compile.at_ms.saturating_sub(compile.took_ms as u64);
        let since_open = opens
            .iter()
            .rfind(|&&open| open <= began)
            .map_or_else(String::new, |open| format!(" (open +{} ms)", began - open));
        let _ = writeln!(
            out,
            "   began +{began} ms{since_open}: {:.1} ms [{}]",
            compile.took_ms, compile.panels
        );
    }
    (compiles, opens)
}

/// The crash loop (user ruling 2026-11-10, `host/panel_restart.rs`): the
/// plugin's tab (in the background: the playlist is the current tab)
/// compiles at most once; while it crashes and is restarted (four times)
/// its panel stays and each restart mounts the same panel again: no
/// compile. Once it hits its start limit its panel goes, compiling nothing.
fn crash_loop_failures(name: &str, compiles: &[Compile], log: &str) -> Vec<String> {
    let mut failures = Vec::new();
    let fixture = format!("{THIRD_PARTY}:sidebar");
    let with = compiles
        .iter()
        .filter(|c| c.panels.contains(&fixture))
        .count();
    if with > 1 {
        failures.push(format!(
            "{name}: {with} compile(s) of its panel (a restart compiled)"
        ));
    }
    // It listens to the player (a `source.state` listener): the host writes to it
    // during playback, also as it crashes. A write to its closed pipe is
    // dropped; it never stops it for good before its restart (it did).
    if log.contains(&format!("({THIRD_PARTY}) transport:")) {
        failures.push(format!(
            "{name}: a send to it failed and stopped it (transport)"
        ));
    }
    let gave_up = log.contains(&format!("supervisor: {THIRD_PARTY} start limit hit"));
    if !gave_up {
        failures.push(format!("{name}: it never hit its start limit"));
    }
    let restarts = log
        .matches("restarts after a fault; its 1 panel(s) stay")
        .count();
    let reused = log
        .matches(&format!(
            "shell: {THIRD_PARTY} mounted its sidebar panel again after its restart (no compile)"
        ))
        .count();
    let _ = writeln_stderr(&format!(
        "   crash loop: {restarts} restart(s) kept the panel, {reused} re-mount(s) without a compile"
    ));
    if restarts < 4 || reused == 0 {
        failures.push(format!(
            "{name}: {restarts} restart(s) kept its panel, {reused} re-mount(s) reused it"
        ));
    }
    failures
}

fn writeln_stderr(line: &str) -> std::io::Result<()> {
    use std::io::Write;
    writeln!(std::io::stderr(), "{line}")
}

/// Every remembered layout: each panel shown at start-up compiles once,
/// before the file given at launch opens; none compiles twice, none once
/// the file opened, nor when a second file opens in the same run.
#[test]
#[ignore = "needs Node 26, libmpv, ffmpeg and the staged built-ins: run with --include-ignored (ci:local)"]
fn startup_compiles_each_shown_panel_once_and_an_open_compiles_nothing() {
    let work = tempfile::tempdir().unwrap();
    let plugins = builtins();
    let first = clip(work.path(), "first.mp4");
    let second = clip(work.path(), "second.mp4");
    let mut failures = Vec::new();
    // A player started before (the plugins' first runs recorded) with each
    // layout, and a first start. A debug build (`npm run ci:local`) compiles
    // about seven times slower: two cases there, with more time; the release
    // build runs all (`cargo test --release --test startup_compiles --
    // --include-ignored`).
    let debug = cfg!(debug_assertions);
    let (warm_up, seconds) = if debug { (20, 45) } else { (10, 24) };
    // The crash loop runs until its start limit (five crashes, restarted
    // after 1, 2, 4 and 8 s): about 20 s after the plugin first starts.
    let crash_seconds = 45;
    let cases = [true, false]
        .into_iter()
        .flat_map(|adjust_open| {
            [ThirdParty::None, ThirdParty::Parked, ThirdParty::Resident]
                .map(|third_party| (true, adjust_open, third_party))
        })
        .chain([
            (false, true, ThirdParty::Resident),
            (true, true, ThirdParty::Crashing),
        ])
        .filter(|&(ran_before, adjust_open, third_party)| {
            !debug
                || (ran_before
                    && adjust_open
                    && matches!(third_party, ThirdParty::None | ThirdParty::Resident))
        });
    for (ran_before, adjust_open, third_party) in cases {
        let profile = Profile::new(&plugins, adjust_open, third_party);
        let name = format!(
            "{}adjust {}, third-party {third_party:?}",
            if ran_before { "" } else { "first start, " },
            if adjust_open { "open" } else { "closed" },
        );
        // One case by its name (`VESTRA_TEST_CASE`), when looking into it.
        if std::env::var("VESTRA_TEST_CASE").is_ok_and(|only| !name.contains(&only)) {
            continue;
        }
        if ran_before {
            profile.run(warm_up, None, None);
        }
        let log = profile.run(
            if third_party == ThirdParty::Crashing {
                crash_seconds
            } else {
                seconds
            },
            Some(&first),
            Some((&second, Duration::from_secs(5))),
        );
        let (compiles, opens) = report(&name, &log);
        if let Some(dir) = std::env::var_os("VESTRA_TEST_LOG_DIR") {
            let file = name.replace([' ', ','], "-");
            std::fs::write(Path::new(&dir).join(format!("{file}.log")), &log).unwrap();
        }
        if opens.len() != 2 {
            failures.push(format!("{name}: {} open(s), not 2", opens.len()));
            continue;
        }
        if third_party == ThirdParty::Crashing {
            // A plugin restarted after a crash is not waited for: the file
            // opens as soon as the others settled (it waited for the restarts
            // before). Its first run is still waited for (it may mount): 1.5
            // to 2.6 s here, against 12 to 17 s before; 8 s leaves room for
            // a loaded machine (a plain case once opened at +3.9 s) and still
            // separates the two.
            if opens[0] >= 8000 {
                failures.push(format!("{name}: the file opened at +{} ms", opens[0]));
            }
            failures.extend(crash_loop_failures(&name, &compiles, &log));
            continue;
        }
        // Each shown panel once (its compiled file is kept for the run);
        // a third-party tab in the background compiles nothing. All before
        // the file opens: nothing compiles once playback began, nor when a
        // file opens later.
        let mut seen = std::collections::BTreeSet::new();
        for compile in &compiles {
            if !seen.insert(compile.panels.as_str()) {
                failures.push(format!("{name}: {} compiled twice", compile.panels));
            }
        }
        if compiles
            .iter()
            .any(|c| c.panels.contains(&format!("{THIRD_PARTY}:")))
        {
            failures.push(format!("{name}: the background third-party tab compiled"));
        }
        let after_open = compiles
            .iter()
            .filter(|c| c.at_ms.saturating_sub(c.took_ms as u64) >= opens[0])
            .count();
        if after_open > 0 {
            failures.push(format!(
                "{name}: {after_open} compile(s) after a file opened"
            ));
        }
        let adjust = compiles
            .iter()
            .filter(|c| c.panels.contains(":adjust"))
            .count();
        // The column left open: its start-up showing is its first opening,
        // which compiles all four built-in tabs (user ruling 2026-11-11).
        if adjust_open {
            for (id, _) in ADJUST_TABS {
                if !seen.contains(format!("{id}:adjust").as_str()) {
                    failures.push(format!(
                        "{name}: the adjust tab of {id} was not compiled at the column's opening"
                    ));
                }
            }
        }
        // The user's ruling: a closed adjust column costs no compile.
        if !adjust_open && adjust > 0 {
            failures.push(format!(
                "{name}: {adjust} compile(s) with adjust panels while the column stayed closed"
            ));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// V2 finding (SDK 1.9.0): a third-party sidebar tab replayed as a
/// placeholder at start-up mounts before the playlist's plugin has started;
/// it became the current tab, so its plugin was woken while the playlist
/// (the remembered current tab) was still on its way. Until the remembered
/// tab mounts no other tab is current: the placeholder is not woken, and the
/// playlist is the current tab once start-up settled.
#[test]
#[ignore = "needs Node 26, libmpv, ffmpeg and the staged built-ins: run with --include-ignored (ci:local)"]
fn a_replayed_placeholder_tab_is_not_woken_while_the_remembered_tab_starts() {
    let plugins = builtins();
    // A debug build starts the fixture's first run late (behind the
    // built-ins' first runs, about seven times slower).
    let (warm_up, seconds) = if cfg!(debug_assertions) {
        (45, 30)
    } else {
        (10, 15)
    };
    let profile = Profile::new(&plugins, false, ThirdParty::Parked);
    // The first run records its plugins (the fixture's tab: a placeholder
    // at the next start). A slow debug start may end before the fixture's
    // first run (it waits behind the built-ins): once more then.
    let recorded = |log: &str| {
        log.contains(&format!("lazy: {THIRD_PARTY} recorded at quit"))
            || log.contains(&format!("lazy: {THIRD_PARTY} parked"))
    };
    let mut warm = profile.run(warm_up, None, None);
    for _ in 0..2 {
        if recorded(&warm) {
            break;
        }
        warm = profile.run(warm_up, None, None);
    }
    assert!(recorded(&warm), "the fixture was never recorded\n{warm}");
    // The sidebar left open on the playlist.
    let path = profile.dir.path().join("ui-state.json");
    let mut state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    state["sidebarSlot"]["visible"] = true.into();
    state["sidebarSlot"]["active"] = "vestra.playlist".into();
    std::fs::write(&path, state.to_string()).unwrap();
    let log = profile.run(seconds, None, None);
    assert!(
        log.contains(&format!("lazy: {THIRD_PARTY} replayed")),
        "the fixture was not replayed as a placeholder\n{log}"
    );
    assert!(
        log.contains("lazy: start-up settled"),
        "start-up never settled\n{log}"
    );
    assert!(
        !log.contains(&format!("tab of {THIRD_PARTY} shown, waking it")),
        "the placeholder tab was woken at start-up\n{log}"
    );
    let state: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        state["sidebarSlot"]["active"], "vestra.playlist",
        "the playlist is no longer the current tab"
    );
}

/// The adjust column's four built-in tabs (Picture, Audio, Subtitles,
/// Playback), in their order, and the command that shows each.
const ADJUST_TABS: [(&str, &str); 4] = [
    ("vestra.video", "video.picture"),
    ("vestra.audio", "audio.workbench"),
    ("vestra.subtitles", "tune"),
    ("vestra.playback", "panel"),
];

/// The adjust column's first opening compiles all its front tabs at once
/// (user ruling 2026-11-11, `Host::preload_front`), each on its own
/// (`ui/panels.rs`), and a first switch to each of them afterwards compiles
/// nothing (only an instance is created; a switch back hands over the kept
/// one); while it is closed, nothing of it compiles. The window's tree is
/// never swapped, so no compile reconfigures its surface (the tab-switch
/// flash of 2026-11-11). The column is opened and the tabs switched through
/// their commands (`VESTRA_TEST_EXECUTE`), as the menus and keys do.
#[test]
#[ignore = "needs Node 26, libmpv, ffmpeg and the staged built-ins: run with --include-ignored (ci:local)"]
fn opening_the_adjust_column_compiles_its_tabs_once_and_switching_compiles_nothing() {
    let work = tempfile::tempdir().unwrap();
    let plugins = builtins();
    let file = clip(work.path(), "first.mp4");
    // A debug build compiles about seven times slower.
    let (warm_up, open, step, seconds) = if cfg!(debug_assertions) {
        (20, 20_000u64, 4_000u64, 50)
    } else {
        (10, 8_000, 2_000, 20)
    };
    let profile = Profile::new(&plugins, false, ThirdParty::None);
    profile.run(warm_up, None, None);
    // Open the column on Picture, then switch to Audio, Subtitles and
    // Playback (each a first showing), and back to Picture.
    let switches = ADJUST_TABS
        .iter()
        .chain(&ADJUST_TABS[..1])
        .enumerate()
        .map(|(i, (id, command))| {
            let at = if i == 0 {
                open
            } else {
                open + 2 * step + i as u64 * step
            };
            format!("{id}/{command}@{at}")
        })
        .collect::<Vec<_>>()
        .join(",");
    let seconds = seconds.to_string();
    let output = profile
        .command(&[
            std::ffi::OsStr::new("--seconds"),
            seconds.as_ref(),
            file.as_os_str(),
        ])
        .env("VESTRA_TEST_EXECUTE", &switches)
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");
    let (compiles, _) = report("adjust opened, then every tab shown", &log);
    let ran = log.matches("lazy: VESTRA_TEST_EXECUTE vestra.").count();
    assert_eq!(ran, 5, "every switch ran\n{log}");
    let before = compiles
        .iter()
        .filter(|c| c.at_ms.saturating_sub(c.took_ms as u64) < open)
        .collect::<Vec<_>>();
    assert!(
        before.iter().all(|c| !c.panels.contains(":adjust")),
        "an adjust panel compiled while the column was closed: {before:#?}"
    );
    let after = compiles
        .iter()
        .filter(|c| c.at_ms.saturating_sub(c.took_ms as u64) >= open)
        .collect::<Vec<_>>();
    // All four at the opening (Picture, the tab it shows, first), in the
    // same tick before the first switch; the switches compile nothing.
    let order = after.iter().map(|c| c.panels.as_str()).collect::<Vec<_>>();
    let expected = ADJUST_TABS
        .iter()
        .map(|(id, _)| format!("{id}:adjust"))
        .collect::<Vec<_>>();
    assert_eq!(order, expected, "{after:#?}");
    let first_switch = open + 3 * step;
    assert!(
        after.iter().all(|c| c.at_ms < first_switch),
        "a compile at a switch: {after:#?}"
    );
}
