//! The player starting itself again after the windows' GPU device was lost
//! (`platform::window_device_lost`; Windows wgpu windows only). The new
//! player is started as `rendering.restart` starts one
//! (`rendering::relaunch_command`); the loss is recorded in the crash guard
//! first, so a second loss within a day makes the next start render in
//! software (round record 2026-10-render-modes §3.5 item 4, §4 M3).
use super::rendering::{RESTARTING, Relaunch, relaunch_command};
use crate::platform;
use crate::playback::render_state::{self, GuardFile, GuardKey};
use serde_json::Value;
use std::sync::atomic::Ordering;

/// The newest `source.state` (any thread may read it: the restart after a
/// lost GPU device starts from the device's callback).
pub(crate) static LAST_SOURCE: std::sync::Mutex<Value> = std::sync::Mutex::new(Value::Null);

/// What a restart after a lost device needs, cloned into the device's
/// callback before the windows exist.
#[derive(Clone)]
pub(crate) struct LostDevice {
    pub relaunch: Relaunch,
    pub guard: GuardFile,
    pub key: GuardKey,
}

/// The windows' GPU device was lost: record it in the crash guard, then
/// start the player again, after this process's teardown, on what it was
/// playing (browsers restore the session after a GPU process loss the same
/// way). Once per process; `rendering.restart` and this share the flag.
///
/// The new player gets the current source if it is a local file, with its
/// position and paused state (`--vestra-resume`), the development plugins
/// and `--silent`; not the launch's files. It waits for this process to end
/// before it claims the single-instance lock (`--vestra-restart`).
pub(crate) fn restart_player(lost: &LostDevice, source: &Value) {
    if RESTARTING.swap(true, Ordering::AcqRel) {
        return;
    }
    let off = lost
        .guard
        .device_lost(lost.key.clone(), render_state::now());
    let Ok(exe) = std::env::current_exe() else {
        restart_log("render: the GPU device was lost; the player could not restart itself");
        return;
    };
    let mut command = relaunch_command(&exe, &lost.relaunch, source);
    // A test switch must not remove the restarted player's device too.
    command.env_remove("VESTRA_TEST_REMOVE_DEVICE");
    restart_log(&format!(
        "render: the GPU device was lost; restarting the player{}",
        if off {
            " with software rendering (the second loss within a day; crash guard)"
        } else {
            ""
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
        let _ = writeln!(
            file,
            "{} pid {}: {line}",
            render_state::now(),
            std::process::id()
        );
    }
}
