//! A player's start-up leaves the export folders of other running players
//! alone. Exports of parts (`media_join.rs`) write their intermediates into
//! `<app cache>/export-parts/.vestra-join-*`, which the start-up cleans of
//! crashed exports' leftovers. Another player of the same user and cache (a
//! second player, a test run) starting meanwhile removed a running export's
//! folder (2026-11-13/15: "joining the parts failed … Impossible to open
//! part-000.mp4"). Now each folder names its owner process; only folders
//! whose owner has ended are removed, and folders without an owner only once
//! a day old.
//!
//! The real host, headless, over a cache under a test folder holding a
//! folder owned by this test process (alive), one by an ended process and
//! one without an owner (new). `VESTRA_TEST_HOST_EXE` runs another build
//! (before and after a change). Needs libmpv; ignored by default, `npm run
//! ci:local` runs it (`--include-ignored`).
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

mod isolation;

/// The player's cache under the isolated `root` (`platform::app_cache_dir`
/// with `isolation::isolate`'s folders).
fn cache_dir(root: &Path) -> PathBuf {
    if cfg!(windows) {
        root.join("local").join("Vestra")
    } else if cfg!(target_os = "macos") {
        root.join("home/Library/Caches/Vestra")
    } else {
        root.join("xdg-cache").join("vestra")
    }
}

/// This process's start mark, as the host writes it in an owner file
/// (`platform::process_start`).
fn own_start() -> String {
    #[cfg(windows)]
    {
        vestra_win32::process_creation_time(std::process::id())
            .unwrap()
            .unwrap()
            .to_string()
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
        let rest = &stat[stat.rfind(')').unwrap() + 1..];
        let start = rest.split_whitespace().nth(19).unwrap();
        let boot = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap();
        format!("{}:{start}", boot.trim())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let output = Command::new("/bin/ps")
            .args(["-o", "lstart=", "-p", &std::process::id().to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

/// The id of a process that has ended.
fn ended_pid() -> u32 {
    let mut child = if cfg!(windows) {
        Command::new("cmd").args(["/C", "exit"]).spawn().unwrap()
    } else {
        Command::new("true").spawn().unwrap()
    };
    let pid = child.id();
    child.wait().unwrap();
    pid
}

#[test]
#[ignore = "needs libmpv: run with --include-ignored (ci:local)"]
fn a_second_player_leaves_a_running_export_folder_alone() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let plugins = root.join("plugins");
    std::fs::create_dir_all(&plugins).unwrap();
    std::fs::write(
        plugins.join("builtin.json"),
        serde_json::json!({"schema":3,"plugins":[]}).to_string(),
    )
    .unwrap();
    let scratch = cache_dir(root).join("export-parts");
    let folder = |name: &str, owner: Option<String>| {
        let path = scratch.join(name);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("part-000.mp4"), b"x").unwrap();
        std::fs::write(path.join("parts.txt"), b"file 'part-000.mp4'\n").unwrap();
        if let Some(owner) = owner {
            std::fs::write(path.join("owner"), owner).unwrap();
        }
        path
    };
    let running = folder(
        ".vestra-join-running",
        Some(format!("{}\n{}\n", std::process::id(), own_start())),
    );
    let ended = folder(
        ".vestra-join-ended",
        Some(format!("{}\nended\n", ended_pid())),
    );
    let new = folder(".vestra-join-new", None);

    let exe = std::env::var_os("VESTRA_TEST_HOST_EXE").map_or_else(
        || PathBuf::from(env!("CARGO_BIN_EXE_vestra-host")),
        PathBuf::from,
    );
    let mut command = Command::new(exe);
    isolation::isolate(&mut command, root)
        .args(["--headless", "--silent", "--seconds", "2"])
        .env("VESTRA_BUILTIN_PLUGINS", &plugins)
        .env("VESTRA_SHARED", "0")
        .env_remove("VESTRA_IDLE_MS")
        .env_remove("VESTRA_TEST_EXECUTE");
    let output = command.output().unwrap();
    let log = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(output.status.success(), "{log}");

    // The start-up cleaned: the ended export's folder is gone ...
    assert!(!ended.exists(), "the ended export's folder stays\n{log}");
    // ... and the running export's folder, and the new one without an
    // owner, are whole.
    for path in [&running, &new] {
        assert!(
            path.join("part-000.mp4").is_file() && path.join("parts.txt").is_file(),
            "{} was removed\n{log}",
            path.display()
        );
    }
}
