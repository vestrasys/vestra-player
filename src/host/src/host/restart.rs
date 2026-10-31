//! The player starting itself again after the windows' GPU device was lost
//! (`platform::window_device_lost`; Windows wgpu windows only). The new
//! player is started as `rendering.restart` starts one
//! (`rendering::relaunch_command`); the loss is recorded in the crash guard
//! first, so a second loss within a day makes the next start render in
//! software (round record 2026-10-render-modes §3.5 item 4, §4 M3).
use super::rendering::{RESTARTING, Relaunch, relaunch_command, restart_refused};
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
    /// This player was started by a restart, at this time: it does not
    /// restart itself again within `rendering::RESTART_COOLDOWN`.
    pub restarted_at: Option<std::time::Instant>,
}

/// The windows' GPU device was lost: record it in the crash guard, then
/// start the player again, after this process's teardown, on what it was
/// playing (browsers restore the session after a GPU process loss the same
/// way). Once per process; `rendering.restart` and this share the flag.
///
/// The new player gets the current source if it is a local file, with its
/// position and paused state (`--vestra-resume`), the development plugins
/// and `--silent`; not the launch's files. It waits for this process to end
/// before it claims the single-instance lock (`--vestra-restart`). A player
/// that was itself restarted less than `RESTART_COOLDOWN` ago only records
/// the loss and does not restart; when the guard turned GPU rendering off,
/// the new player is told to render in software even if `VESTRA_RENDER=gpu`
/// is set (it would otherwise lose the device again, forever).
pub(crate) fn restart_player(lost: &LostDevice, source: &Value) {
    if RESTARTING.swap(true, Ordering::AcqRel) {
        return;
    }
    let off = lost
        .guard
        .device_lost(lost.key.clone(), render_state::now());
    if let Some(why) = restart_refused(false, lost.restarted_at, std::time::Instant::now()) {
        restart_log(&format!(
            "render: the GPU device was lost; not restarting ({why})"
        ));
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        restart_log("render: the GPU device was lost; the player could not restart itself");
        return;
    };
    let mut command = command_after_loss(&exe, lost, source, off);
    restart_log(&format!(
        "render: the GPU device was lost; restarting the player{}",
        if off {
            " with software rendering (the second loss within a day; crash guard)"
        } else {
            ""
        }
    ));
    restart_log(&format!("render: restarting with {command:?}"));
    // As `rendering.restart`: the new player may take the foreground, and
    // this one takes no more files from later launches.
    platform::allow_handoff_foreground();
    crate::stop_handoffs();
    match command.spawn() {
        Ok(child) => restart_log(&format!(
            "render: the restarted player is process {}",
            child.id()
        )),
        Err(error) => restart_log(&format!("render: the restart failed: {error}")),
    }
}

/// The new player's command line after a device loss (`off`: the crash
/// guard turned GPU rendering off).
fn command_after_loss(
    exe: &std::path::Path,
    lost: &LostDevice,
    source: &Value,
    off: bool,
) -> std::process::Command {
    let mut command = relaunch_command(exe, &lost.relaunch, source);
    // A test switch must not remove the restarted player's device too.
    command.env_remove("VESTRA_TEST_REMOVE_DEVICE");
    if off {
        // `VESTRA_RENDER=gpu` overrides the guard; the child must not.
        command.env("VESTRA_RENDER", "software");
    }
    command
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::rendering::RESTART_COOLDOWN;

    fn lost(restarted_at: Option<std::time::Instant>) -> LostDevice {
        LostDevice {
            relaunch: Relaunch::default(),
            guard: GuardFile::new(None),
            key: GuardKey::default(),
            restarted_at,
        }
    }
    fn env(command: &std::process::Command, name: &str) -> Option<Option<String>> {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
    }

    /// M3 review M1: with `VESTRA_RENDER=gpu` a device that keeps getting
    /// lost must not restart the player on the GPU forever.
    #[test]
    fn once_the_guard_turned_gpu_off_the_new_player_renders_in_software() {
        let path = std::path::Path::new("player");
        let on = command_after_loss(path, &lost(None), &Value::Null, false);
        assert_eq!(env(&on, "VESTRA_RENDER"), None, "the environment as it is");
        assert_eq!(env(&on, "VESTRA_TEST_REMOVE_DEVICE"), Some(None), "removed");
        let off = command_after_loss(path, &lost(None), &Value::Null, true);
        assert_eq!(env(&off, "VESTRA_RENDER"), Some(Some("software".into())));
        let args: Vec<_> = off.get_args().collect();
        assert_eq!(args[0], "--vestra-restart");
    }

    #[test]
    fn a_restarted_player_does_not_restart_again_within_the_cooldown() {
        let start = std::time::Instant::now();
        let restarted = lost(Some(start));
        let soon = start + std::time::Duration::from_secs(5);
        assert!(restart_refused(false, restarted.restarted_at, soon).is_some());
        let later = start + RESTART_COOLDOWN;
        assert!(restart_refused(false, restarted.restarted_at, later).is_none());
        assert!(restart_refused(false, lost(None).restarted_at, soon).is_none());
    }
}
