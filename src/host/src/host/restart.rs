//! The player starting itself again after the windows' GPU device was lost
//! (`platform::window_device_lost`; Windows wgpu windows only).
use crate::launch;
use crate::platform;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The newest `source.state` (any thread may read it: the restart after a
/// lost GPU device starts from the device's callback).
pub(crate) static LAST_SOURCE: std::sync::Mutex<Value> = std::sync::Mutex::new(Value::Null);
/// The restart after a lost GPU device was started (once per process).
static RESTARTING: AtomicBool = AtomicBool::new(false);

/// The windows' GPU device was lost (`platform::window_device_lost`): start
/// the player again, after this process's teardown, on what it was playing
/// (browsers restore the session after a GPU process loss the same way).
///
/// - The new player gets the current source if it is a local file, with
///   its position and paused state (`--vestra-resume`), the development
///   plugins and `--silent`; not the launch's files. Plugin state (the
///   playlist) restores itself.
/// - It waits for this process to end before it claims the single-instance
///   lock, and does not run without it (`--vestra-restart`).
/// - A second loss in a row restarts with OpenGL windows, so a device that
///   keeps failing cannot make a restart loop.
///
/// Internal arguments, not the environment: nothing leaks to the plugin
/// processes the new player starts.
pub(crate) fn restart_player(
    restarted: Option<launch::Restart>,
    directories: &[PathBuf],
    silent: bool,
    source: &Value,
) {
    if RESTARTING.swap(true, Ordering::AcqRel) {
        return;
    }
    let count = restarted.map_or(0, |restart| restart.count) + 1;
    let restart = launch::Restart {
        parent: std::process::id(),
        count,
        opengl_windows: count >= 2,
    };
    let Ok(exe) = std::env::current_exe() else {
        diag!("render: the GPU device was lost; the player could not restart itself");
        return;
    };
    let mut command = std::process::Command::new(exe);
    command.args(["--vestra-restart".to_string(), restart.arg()]);
    // A test switch must not remove the restarted player's device too.
    command.env_remove("VESTRA_TEST_REMOVE_DEVICE");
    for directory in directories {
        command.arg("--plugin").arg(directory);
    }
    if silent {
        command.arg("--silent");
    }
    let (file, resume) = resume_point(source);
    if let Some(resume) = resume {
        command.args(["--vestra-resume".to_string(), resume.arg()]);
    }
    if let Some(file) = &file {
        command.arg("--").arg(file);
    }
    restart_log(&format!(
        "render: the GPU device was lost; restarting the player{}{}",
        if restart.opengl_windows {
            " with OpenGL windows"
        } else {
            ""
        },
        match (&file, &source["source"]) {
            (Some(file), _) => format!(
                " at {} in {}",
                resume.map_or(0., |r| r.seconds),
                file.display()
            ),
            (None, Value::String(other)) => format!(" without {other} (not a local file)"),
            _ => String::new(),
        }
    ));
    restart_log(&format!("render: restarting with {command:?}"));
    match command.spawn() {
        Ok(child) => restart_log(&format!(
            "render: the restarted player is process {}",
            child.id()
        )),
        Err(error) => restart_log(&format!("render: the restart failed: {error}")),
    }
}

/// A `render:` line about a restart: to stderr, and appended to
/// `logs/restart.log` in the application data directory, so a restarted
/// player that exits early (its stderr goes nowhere) still says why.
pub(crate) fn restart_log(line: &str) {
    diag!("{line}");
    let Some(directory) = platform::app_data_dir().map(|dir| dir.join("logs")) else {
        return;
    };
    let _ = std::fs::create_dir_all(&directory);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("restart.log"))
    {
        use std::io::Write;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let _ = writeln!(file, "{stamp} pid {}: {line}", std::process::id());
    }
}

/// The local file `source` (a `source.state`) plays or paused in, and
/// where; nothing for no source, an ended one, or one that is not a file.
fn resume_point(source: &Value) -> (Option<PathBuf>, Option<launch::Resume>) {
    let file = source["source"]
        .as_str()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute() && Path::is_file(path));
    let live = matches!(source["phase"].as_str(), Some("ready" | "loading"));
    let Some(file) = file.filter(|_| live) else {
        return (None, None);
    };
    let resume = launch::Resume {
        seconds: source["position"]
            .as_f64()
            .filter(|s| s.is_finite() && *s >= 0.)
            .unwrap_or(0.),
        paused: source["paused"].as_bool().unwrap_or(false),
    };
    (Some(file), Some(resume))
}
