//! The player starting itself again after the windows' GPU device was lost
//! (`platform::window_device_lost`; Windows wgpu windows only). The new
//! player is started as `rendering.restart` starts one
//! (`rendering::relaunch_command`); the loss is recorded in the crash guard
//! first, so a second loss within a day makes the next start render in
//! software (round record 2026-10-render-modes §3.5 item 4, §4 M3).
use super::rendering::{RESTARTING, Relaunch, relaunch_command, restart_refused};
use crate::platform;
use crate::playback::render_state::{self, GuardFile, Keys};
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
    pub keys: Keys,
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
/// before it claims the single-instance lock (`--vestra-restart`). The new
/// player is told to render in software (`VESTRA_RENDER=software`, which
/// also beats a `VESTRA_RENDER=gpu` that would otherwise lose the device
/// again, forever) when the guard turned GPU rendering off, or when this
/// player was itself restarted less than `RESTART_COOLDOWN` ago (a GPU
/// relaunch is refused then; the player never just disappears).
pub(crate) fn restart_player(lost: &LostDevice, source: &Value) {
    if RESTARTING.swap(true, Ordering::AcqRel) {
        return;
    }
    // The shader guard's pending `applying` reaches the disk first.
    super::diagnostics::persist_shader_guard();
    let off = lost.guard.device_lost(&lost.keys, render_state::now());
    let why = software_after_loss(lost, off, std::time::Instant::now());
    let Ok(exe) = std::env::current_exe() else {
        restart_logged("render: the GPU device was lost; the player could not restart itself");
        return;
    };
    let mut command = command_after_loss(&exe, lost, source, why.is_some());
    restart_logged(&format!(
        "render: the GPU device was lost; restarting the player{}",
        why.map(|why| format!(" with software rendering ({why})"))
            .unwrap_or_default()
    ));
    restart_log(&format!("render: restarting with {command:?}"));
    // As `rendering.restart`: the new player may take the foreground, and
    // this one takes no more files from later launches.
    platform::allow_handoff_foreground();
    crate::stop_handoffs();
    match command.spawn() {
        Ok(child) => {
            restart_logged(&format!(
                "render: the restarted player is process {}",
                child.id()
            ));
            exit_if_stuck(EXIT_AFTER);
        }
        Err(error) => restart_logged(&format!("render: the restart failed: {error}")),
    }
}

/// How long this player may take to end after a device loss started the new
/// one, which waits 20 s for it (`main.rs`) and shows nothing until then.
/// What must survive is saved before the teardown: the crash guard and the
/// resume position here, the UI state and the lazy-activation records first
/// thing after the event loop (`main.rs`); clean teardowns after a loss took
/// 1 to 5 s (restart.log of the M3 and 0.5.19 hardware tests). Up to host
/// 0.6.3 this was 15 s, which a teardown stuck in the driver used in full
/// (0.6.2, shader upscaling at 4K: a black, unresponsive window for 15 s).
const EXIT_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// The step of the teardown this process is in, for the line that says it
/// was ended ([`teardown_step`]).
static TEARDOWN: std::sync::Mutex<&str> = std::sync::Mutex::new("the device-loss callback");

/// The teardown reached `step` (`main.rs`, after the event loop): if the
/// process has to be ended after a device loss, restart.log says where it
/// was stuck.
pub(crate) fn teardown_step(step: &'static str) {
    *TEARDOWN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = step;
}

/// The restart.log line of a player ended after a device loss.
fn stuck_line(after: std::time::Duration, step: &str) -> String {
    format!(
        "render: the player did not end {} s after the device loss (teardown at: {step}); ending it",
        after.as_secs()
    )
}

/// After a device loss or a GPU stall (`vestra-gpu-render`'s watchdog) the
/// teardown can hang in the driver, the render thread being stuck in it;
/// the new player would then wait for this one and the lock (round record
/// 2026-10-render-modes §10 L6). End the process if it is still there
/// after `after`.
fn exit_if_stuck(after: std::time::Duration) {
    let _ = std::thread::Builder::new()
        .name("vestra-exit-after-loss".into())
        .spawn(move || {
            std::thread::sleep(after);
            let step = *TEARDOWN
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            restart_logged(&stuck_line(after, step));
            std::process::exit(3);
        });
}

/// Why the player restarted after a device loss renders in software, if it
/// does: the crash guard turned GPU rendering off (`off`), or this player
/// was itself restarted within the cooldown, where a GPU relaunch is
/// refused (`rendering::restart_refused`).
fn software_after_loss(lost: &LostDevice, off: bool, now: std::time::Instant) -> Option<String> {
    if off {
        Some("the second loss within a day; crash guard".into())
    } else {
        restart_refused(false, lost.restarted_at, now).map(|why| format!("{why}; no GPU relaunch"))
    }
}

/// The new player's command line after a device loss (`off`: it renders in
/// software).
fn command_after_loss(
    exe: &std::path::Path,
    lost: &LostDevice,
    source: &Value,
    off: bool,
) -> std::process::Command {
    let mut command = relaunch_command(exe, &lost.relaunch, source);
    // A test switch must not remove the restarted player's device too.
    command.env_remove("VESTRA_TEST_REMOVE_DEVICE");
    command.env_remove("VESTRA_TEST_EXECUTE");
    command.env_remove("VESTRA_TEST_GPU_STALL");
    if off {
        // `VESTRA_RENDER=gpu` overrides the guard; the child must not.
        command.env("VESTRA_RENDER", "software");
    }
    command
}

/// `restart_log` that also goes to the host log (`diag.rs`): only for lines
/// without the restart's command line (it names the file played).
pub(crate) fn restart_logged(line: &str) {
    logged!("{line}");
    append_restart_log(line);
}

/// A `render:` line about a restart: to stderr, and appended to
/// `logs/restart.log` in the application data directory, so a restarted
/// player that exits early (its stderr goes nowhere) still says why.
pub(crate) fn restart_log(line: &str) {
    diag!("{line}");
    append_restart_log(line);
}

fn append_restart_log(line: &str) {
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
            keys: Keys::default(),
            restarted_at,
        }
    }
    fn env(command: &std::process::Command, name: &str) -> Option<Option<String>> {
        command
            .get_envs()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.map(|value| value.to_string_lossy().into_owned()))
    }

    /// 0.6.2 shader-upscaling loss: a teardown stuck in the driver left a
    /// black, unresponsive window for 15 s. The process is now ended well
    /// before the restarted player stops waiting for it (20 s), and the line
    /// says which teardown step it was stuck in.
    #[test]
    fn a_stuck_teardown_is_ended_soon_and_says_where_it_was() {
        assert!(EXIT_AFTER <= std::time::Duration::from_secs(5));
        assert!(EXIT_AFTER < std::time::Duration::from_secs(20) / 2);
        teardown_step("hiding the main window");
        let step = *TEARDOWN.lock().unwrap();
        let line = stuck_line(EXIT_AFTER, step);
        assert_eq!(
            line,
            "render: the player did not end 5 s after the device loss (teardown at: hiding the main window); ending it"
        );
    }

    /// M3 review M1: with `VESTRA_RENDER=gpu` a device that keeps getting
    /// lost must not restart the player on the GPU forever.
    #[test]
    fn once_the_guard_turned_gpu_off_the_new_player_renders_in_software() {
        let path = std::path::Path::new("player");
        let on = command_after_loss(path, &lost(None), &Value::Null, false);
        assert_eq!(env(&on, "VESTRA_RENDER"), None, "the environment as it is");
        assert_eq!(env(&on, "VESTRA_TEST_REMOVE_DEVICE"), Some(None), "removed");
        assert_eq!(env(&on, "VESTRA_TEST_EXECUTE"), Some(None), "removed");
        assert_eq!(env(&on, "VESTRA_TEST_GPU_STALL"), Some(None), "removed");
        let off = command_after_loss(path, &lost(None), &Value::Null, true);
        assert_eq!(env(&off, "VESTRA_RENDER"), Some(Some("software".into())));
        let args: Vec<_> = off.get_args().collect();
        assert_eq!(args[0], "--vestra-restart");
    }

    /// M3 re-review N1: a loss within the cooldown relaunches in software;
    /// the player never just exits.
    #[test]
    fn a_restarted_player_relaunches_in_software_within_the_cooldown() {
        let start = std::time::Instant::now();
        let restarted = lost(Some(start));
        let soon = start + std::time::Duration::from_secs(5);
        let why = software_after_loss(&restarted, false, soon);
        assert!(
            why.as_deref().is_some_and(|why| why.contains("30 seconds")),
            "{why:?}"
        );
        let later = start + RESTART_COOLDOWN;
        assert_eq!(
            software_after_loss(&restarted, false, later),
            None,
            "GPU again"
        );
        assert_eq!(software_after_loss(&lost(None), false, soon), None);
        assert!(
            software_after_loss(&lost(None), true, soon).is_some(),
            "guard off"
        );
        // Within the cooldown, guard crashed or not: software, never no restart.
        for off in [false, true] {
            let why = software_after_loss(&restarted, off, soon);
            let command = command_after_loss(
                std::path::Path::new("player"),
                &restarted,
                &Value::Null,
                why.is_some(),
            );
            assert_eq!(
                env(&command, "VESTRA_RENDER"),
                Some(Some("software".into())),
                "{off}"
            );
        }
    }
}
